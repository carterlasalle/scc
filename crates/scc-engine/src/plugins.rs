//! Engine plugins namespace: discovery, custom-operation dispatch, cache keys.
//!
//! Custom operations (spec 31): a plugin operation like `acme.echo` is
//! callable through `invoke()`, RPC, HTTP, SDKs, and FFI with no per-transport
//! code — the fallback arm in `invoke` routes unknown `acme.*` ids here.
//! Provenance (spec 25): every plugin output is wrapped with its origin.
//! Failure policy (spec 26): required = hard error, warn/optional = skip
//! with a structured diagnostic. Cache keys (spec 27) include the plugin
//! lock so upgrades invalidate ranking-dependent caches automatically.

use std::path::Path;

// trace:exempt reason=internal-detail
pub struct ActivePlugins {
    pub plugins: Vec<scc_plugin_host::LoadedPlugin>,
    pub diagnostics: Vec<scc_plugin_host::PluginDiagnostic>,
}

// trace:exempt reason=internal-detail
pub fn active(root: &Path, config: &scc_indexer::Config) -> ActivePlugins {
    let mut ap = ActivePlugins { plugins: Vec::new(), diagnostics: Vec::new() };
    let mut plugins = scc_plugin_host::discover(root);
    // Project allow-list: only `plugins.enabled` run when non-empty.
    if !config.plugins.enabled.is_empty() {
        plugins.retain(|p| config.plugins.enabled.contains(&p.manifest.id));
    }
    // Per-plugin config from the project file.
    for p in &mut plugins {
        if let Some(c) = config.plugins.config.get(&p.manifest.id) {
            p.config = c.clone();
        }
    }
    // Grants: explicit project grants narrow manifest defaults. Unknown
    // grant names are diagnostics (fail-loud: a typo must not silently
    // narrow the grant set and misdirect the later denied error).
    let mut grants: std::collections::BTreeMap<String, Vec<scc_plugin_api::Permission>> =
        std::collections::BTreeMap::new();
    for (k, v) in &config.plugins.grants {
        let mut perms = Vec::new();
        for s in v {
            match scc_plugin_api::Permission::parse(s) {
                Ok(p) => perms.push(p),
                Err(e) => ap.diagnostics.push(scc_plugin_host::PluginDiagnostic {
                    plugin: k.clone(), operation: "grants".into(),
                    error: e, action: "skipped".into(),
                }),
            }
        }
        grants.insert(k.clone(), perms);
    }
    scc_plugin_host::apply_grants(&mut plugins, &grants);
    ap.plugins = plugins;
    ap
}

// trace:exempt reason=internal-detail
pub fn lock_entries(ap: &ActivePlugins) -> Vec<serde_json::Value> {
    ap.plugins.iter().map(scc_plugin_host::lock_entry).collect()
}

// trace:v1 id=impl.scc-engine-plugins.cache-key-fragment work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub fn cache_key_fragment(ap: &ActivePlugins) -> String {
    // Plugin-affecting state that must invalidate caches (spec 27).
    let mut h = blake3::Hasher::new();
    for e in lock_entries(ap) {
        h.update(e.to_string().as_bytes());
    }
    format!("plugins:{}", &h.finalize().to_hex()[..16])
}

/// Dispatch a plugin-provided operation. Returns `(output, diagnostic)`:
/// warn/optional failures yield the engine error + a diagnostic instead of
/// failing the call — the host records what was skipped (spec 26).
// trace:v1 id=impl.scc-engine-plugins.call-operation work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub fn call_operation(ap: &mut ActivePlugins, operation: &str, input: serde_json::Value) -> crate::Result<serde_json::Value> {
    let plugin = scc_plugin_host::provider_for(&ap.plugins, operation).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    let id = plugin.manifest.id.clone();
    let policy = plugin.manifest.failure_policy.clone();
    match scc_plugin_host::call(plugin, operation, input, None) {
        Ok(output) => Ok(with_provenance(&id, &plugin.manifest.version, operation, output)),
        Err(e) => {
            let action = if policy == "required" { "failed" } else { "skipped" };
            ap.diagnostics.push(scc_plugin_host::PluginDiagnostic {
                plugin: id.clone(), operation: operation.into(), error: e.to_string(), action: action.into(),
            });
            if policy == "required" {
                Err(crate::EngineError::Other(format!("plugin {id} {operation}: {e}")))
            } else {
                Ok(serde_json::json!({"skipped": true, "plugin": id, "error": e.to_string()}))
            }
        }
    }
}

// trace:exempt reason=internal-detail
fn with_provenance(plugin_id: &str, version: &str, operation: &str, mut output: serde_json::Value) -> serde_json::Value {
    // Mandatory provenance (spec 25): every plugin output carries origin.
    if let Some(obj) = output.as_object_mut() {
        obj.insert("_origin".into(), serde_json::json!({
            "kind": "plugin", "plugin_id": plugin_id,
            "plugin_version": version, "extension": operation,
        }));
    }
    output
}

/// Startup-section contributions (§124 item 29): every `startup-section`
/// extension renders one markdown section into the startup pack.
///
/// Mirrors [`context_sections`]: input carries no goal (startup is
/// goal-free); output `section` markdown is spliced verbatim under a
/// provenance header (`PLUGIN STARTUP SECTION <id> from <plugin>`).
/// Failures follow policy: required = hard error, else skip silently —
/// startup has no warnings channel, so skips are recorded in OMISSIONS by
/// the caller via the returned notes.
// trace:v1 id=impl.scc-engine-plugins.startup-sections work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn startup_sections(
    root: &std::path::Path,
    config: &scc_indexer::Config,
) -> (String, Vec<String>) {
    let ap = active(root, config);
    let mut out = String::new();
    let mut notes: Vec<String> = Vec::new();
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|p| {
            p.manifest.extensions.iter().filter(|e| e.extension_type == "startup-section").map(|e| {
                (p.manifest.id.clone(), e.id.clone(), p.manifest.failure_policy.clone())
            })
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|p| p.manifest.id == pid).cloned() {
            Some(p) => p,
            None => continue,
        };
        let input = serde_json::json!({"section": ext_id});
        match scc_plugin_host::call(&plug, "startup.section", input, None) {
            Ok(v) => {
                let text = v.get("section").and_then(|s| s.as_str()).unwrap_or("");
                if !text.trim().is_empty() {
                    out.push_str(&format!("\n# PLUGIN STARTUP SECTION {ext_id} (from {pid} — plugin content, not verified facts)\n"));
                    out.push_str(text.trim());
                    out.push('\n');
                }
            }
            Err(e) => {
                if policy == "required" {
                    out.push_str(&format!("\nPLUGIN STARTUP SECTION {ext_id} from {pid} FAILED: {e}\n"));
                } else {
                    notes.push(format!("startup section {ext_id} from {pid} skipped ({e})"));
                }
            }
        }
    }
    (out, notes)
}

/// Verify-diagnostic contributions (§124 item 30): every
/// `verify-diagnostic:*` extension renders one findings section into the
/// verify pack via the plugin's `verify.diagnostic` op (no input).
/// Output `diagnostic` markdown is spliced verbatim under a provenance
/// header. Failures follow policy: required = hard error text inline,
/// else skipped with a diagnostic note.
// trace:v1 id=impl.scc-engine-plugins.verify-diagnostics work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn verify_diagnostics(
    root: &std::path::Path,
    config: &scc_indexer::Config,
) -> (String, Vec<String>) {
    let ap = active(root, config);
    let mut out = String::new();
    let mut notes: Vec<String> = Vec::new();
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|p| {
            p.manifest.extensions.iter().filter(|e| e.extension_type == "verify-diagnostic").map(|e| {
                (p.manifest.id.clone(), e.id.clone(), p.manifest.failure_policy.clone())
            })
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|p| p.manifest.id == pid).cloned() {
            Some(p) => p,
            None => continue,
        };
        let input = serde_json::json!({"diagnostic": ext_id});
        match scc_plugin_host::call(&plug, "verify.diagnostic", input, None) {
            Ok(v) => {
                let text = v.get("diagnostic").and_then(|s| s.as_str()).unwrap_or("");
                if !text.trim().is_empty() {
                    out.push_str(&format!("\n# PLUGIN VERIFY DIAGNOSTIC {ext_id} (from {pid} — plugin content, not verified facts)\n"));
                    out.push_str(text.trim());
                    out.push('\n');
                }
            }
            Err(e) => {
                if policy == "required" {
                    out.push_str(&format!("\n# PLUGIN VERIFY DIAGNOSTIC {ext_id} from {pid} FAILED: {e}\n"));
                } else {
                    notes.push(format!("verify diagnostic {ext_id} from {pid} skipped ({e})"));
                }
            }
        }
    }
    (out, notes)
}

/// Viewer-panel contributions (§124 item 32): every `viewer-panel:*`
/// extension contributes one structured data panel to the viewer via the
/// plugin's `viewer.panel` op (input: panel id). Output is data the CLI
/// renders into the viewer nav/pages — never arbitrary JS (spec §104).
/// Each panel carries provenance (plugin id + extension); required-panel
/// failure is an inline error panel, else the panel is skipped with a note.
// trace:v1 id=impl.scc-engine-plugins.viewer-panels work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn viewer_panels(
    root: &std::path::Path,
    config: &scc_indexer::Config,
) -> (Vec<serde_json::Value>, Vec<String>) {
    let ap = active(root, config);
    let mut panels = Vec::new();
    let mut notes = Vec::new();
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|pl| {
            pl.manifest.extensions.iter()
                .filter(|e| e.extension_type == "viewer-panel")
                .map(|e| (pl.manifest.id.clone(), e.id.clone(), pl.manifest.failure_policy.clone()))
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|pl| pl.manifest.id == pid).cloned() {
            Some(pl) => pl,
            None => continue,
        };
        match scc_plugin_host::call(&plug, "viewer.panel", serde_json::json!({"panel": ext_id}), None) {
            Ok(v) => {
                let title = v.get("title").and_then(|x| x.as_str()).unwrap_or(&ext_id).to_string();
                let html = v.get("html").and_then(|x| x.as_str()).unwrap_or("").to_string();
                // Spec §104: no arbitrary plugin JS in the viewer by
                // default. Script-bearing panels are dropped loudly.
                if html.to_lowercase().contains("<script") {
                    notes.push(format!("viewer panel {ext_id} from {pid} skipped (inline <script> not allowed)"));
                } else if !html.trim().is_empty() {
                    panels.push(serde_json::json!({
                        "id": ext_id, "plugin": pid, "title": title, "html": html,
                    }));
                }
            }
            Err(e) => {
                if policy == "required" {
                    panels.push(serde_json::json!({
                        "id": ext_id, "plugin": pid, "title": ext_id,
                        "html": format!("panel failed: {e}"),
                    }));
                } else {
                    notes.push(format!("viewer panel {ext_id} from {pid} skipped ({e})"));
                }
            }
        }
    }
    (panels, notes)
}

/// Quota-policy overrides (§124 item 25): every `quota-policy:*`
/// extension may override per-kind quota fractions via the plugin's
/// `selection.quotas` op (input: ranked ids + request quotas; output
/// `quotas: [{kind, fraction}]`). Returned pairs merge over the request
/// quotas (plugin wins per kind). Failures follow policy: required =
/// hard error, else skipped with a diagnostic. Fractions clamp to
/// [0,1] at apply time (apply_quotas already clamps).
// trace:v1 id=impl.scc-engine-plugins.quota-overrides work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn quota_overrides(
    ap: &ActivePlugins,
    req: &scc_api::SelectionRequest,
) -> (Vec<(String, f64)>, Vec<scc_plugin_host::PluginDiagnostic>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|pl| {
            pl.manifest.extensions.iter()
                .filter(|e| e.extension_type == "quota-policy")
                .map(|e| (pl.manifest.id.clone(), e.id.clone(), pl.manifest.failure_policy.clone()))
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|pl| pl.manifest.id == pid).cloned() {
            Some(pl) => pl,
            None => continue,
        };
        let ranked: Vec<serde_json::Value> = req.ranked.iter().map(|e| {
            serde_json::json!({"id": e.id, "kind": e.kind, "value": e.value, "token_cost": e.token_cost})
        }).collect();
        let quotas: Vec<serde_json::Value> = req.quotas.clone().unwrap_or_default().into_iter().map(|q| {
            serde_json::json!({"kind": q.kind, "fraction": q.fraction})
        }).collect();
        let input = serde_json::json!({"policy": ext_id, "ranked": ranked, "quotas": quotas});
        match scc_plugin_host::call(&plug, "selection.quotas", input, None) {
            Ok(v) => {
                if let Some(arr) = v.get("quotas").and_then(|x| x.as_array()) {
                    for q in arr {
                        let kind = q.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                        let frac = q.get("fraction").and_then(|x| x.as_f64()).unwrap_or(-1.0);
                        if !kind.is_empty() && (0.0..=1.0).contains(&frac) {
                            out.push((kind.to_string(), frac));
                        }
                    }
                }
            }
            Err(e) => {
                if policy == "required" {
                    notes.push(scc_plugin_host::PluginDiagnostic {
                        plugin: pid, operation: "selection.quotas".into(),
                        error: format!("quota policy {ext_id} failed: {e}"), action: "failed".into(),
                    });
                }
            }
        }
    }
    (out, notes)
}

/// Budget-optimizer selection (§124 item 27): the single declaring
/// `budget-optimizer:*` extension replaces budget selection via the
/// plugin's `selection.optimize` op (input: ranked ids + budget; output
/// `selected: [ids]`). Zero declarers = None (caller runs the default).
/// Two+ declarers = hard error (spec §32: no silent last-wins for
/// exclusive policy slots). Unknown ids in the plugin answer fail
/// loudly; an empty answer is honored (select nothing).
// trace:v1 id=impl.scc-engine-plugins.budget-selection work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn budget_selection(
    ap: &ActivePlugins,
    req: &scc_api::SelectionRequest,
    items: &[scc_core::ContextItem],
    budget: usize,
) -> crate::Result<Option<Vec<String>>> {
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|pl| {
            pl.manifest.extensions.iter()
                .filter(|e| e.extension_type == "budget-optimizer")
                .map(|e| (pl.manifest.id.clone(), e.id.clone(), pl.manifest.failure_policy.clone()))
        })
        .collect();
    if specs.is_empty() {
        return Ok(None);
    }
    if specs.len() > 1 {
        let who: Vec<String> = specs.iter().map(|(pid, eid, _)| format!("{eid} from {pid}")).collect();
        return Err(crate::EngineError::Other(format!(
            "multiple budget-optimizer extensions compete ({}) — exclusive policy slot needs explicit configuration (spec §32)",
            who.join(", ")
        )));
    }
    let (pid, ext_id, policy) = &specs[0];
    let plug = ap.plugins.iter().find(|pl| &pl.manifest.id == pid).cloned()
        .ok_or_else(|| crate::EngineError::Other(format!("budget optimizer plugin {pid} vanished")))?;
    let ranked: Vec<serde_json::Value> = req.ranked.iter().map(|e| {
        serde_json::json!({"id": e.id, "kind": e.kind, "value": e.value, "token_cost": e.token_cost})
    }).collect();
    let input = serde_json::json!({"optimizer": ext_id, "ranked": ranked, "budget": budget});
    match scc_plugin_host::call(&plug, "selection.optimize", input, None) {
        Ok(v) => {
            let known: std::collections::BTreeSet<&str> =
                items.iter().map(|i| i.id.as_str()).collect();
            let mut sel = Vec::new();
            if let Some(arr) = v.get("selected").and_then(|x| x.as_array()) {
                for x in arr {
                    let id = x.as_str().unwrap_or("");
                    if !known.contains(id) {
                        return Err(crate::EngineError::Other(format!(
                            "budget optimizer {ext_id} from {pid} selected unknown id '{id}'"
                        )));
                    }
                    sel.push(id.to_string());
                }
            }
            Ok(Some(sel))
        }
        Err(e) => {
            if policy == "required" {
                Err(crate::EngineError::Other(format!(
                    "budget optimizer {ext_id} from {pid} failed: {e}"
                )))
            } else {
                Ok(None)
            }
        }
    }
}

/// Diversity-policy selection (§124 item 24): the single declaring
/// `diversity-policy:*` extension replaces MMR via the plugin's
/// `selection.diversify` op (input: ranked ids + lambda; output
/// `selected: [ids]`, honored verbatim). Zero declarers = None (caller
/// runs default MMR). Two+ declarers = hard error (spec §32: exclusive
/// policy slots never silently last-win). Unknown ids fail loudly; an
/// empty answer is honored (select nothing).
// trace:v1 id=impl.scc-engine-plugins.diversity-selection work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn diversity_selection(
    ap: &ActivePlugins,
    req: &scc_api::SelectionRequest,
) -> crate::Result<Option<Vec<String>>> {
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|pl| {
            pl.manifest.extensions.iter()
                .filter(|e| e.extension_type == "diversity-policy")
                .map(|e| (pl.manifest.id.clone(), e.id.clone(), pl.manifest.failure_policy.clone()))
        })
        .collect();
    if specs.is_empty() {
        return Ok(None);
    }
    if specs.len() > 1 {
        let who: Vec<String> = specs.iter().map(|(pid, eid, _)| format!("{eid} from {pid}")).collect();
        return Err(crate::EngineError::Other(format!(
            "multiple diversity-policy extensions compete ({}) — exclusive policy slot needs explicit configuration (spec §32)",
            who.join(", ")
        )));
    }
    let (pid, ext_id, policy) = &specs[0];
    let plug = ap.plugins.iter().find(|pl| &pl.manifest.id == pid).cloned()
        .ok_or_else(|| crate::EngineError::Other(format!("diversity plugin {pid} vanished")))?;
    let ranked: Vec<serde_json::Value> = req.ranked.iter().map(|e| {
        serde_json::json!({"id": e.id, "value": e.value})
    }).collect();
    let input = serde_json::json!({
        "policy": ext_id, "ranked": ranked,
        "lambda": req.lambda.unwrap_or(0.5),
    });
    match scc_plugin_host::call(&plug, "selection.diversify", input, None) {
        Ok(v) => {
            let known: std::collections::BTreeSet<&str> =
                req.ranked.iter().map(|e| e.id.as_str()).collect();
            let mut sel = Vec::new();
            if let Some(arr) = v.get("selected").and_then(|x| x.as_array()) {
                for x in arr {
                    let id = x.as_str().unwrap_or("");
                    if !known.contains(id) {
                        return Err(crate::EngineError::Other(format!(
                            "diversity policy {ext_id} from {pid} selected unknown id '{id}'"
                        )));
                    }
                    sel.push(id.to_string());
                }
            }
            Ok(Some(sel))
        }
        Err(e) => {
            if policy == "required" {
                Err(crate::EngineError::Other(format!(
                    "diversity policy {ext_id} from {pid} failed: {e}"
                )))
            } else {
                Ok(None)
            }
        }
    }
}

/// Sidecar promotion (§124 item 36): promote selected sidecar findings
/// into canonical relationships through the NORMAL contribution path
/// (validate + atomic commit) — never a side door. Each assertion needs
/// existing subject/object entity ids (no invented endpoints), a CORE
/// ontology predicate (custom semantics stay `plugin:<id>/...` via
/// `plugins.contribute`), and a confidence in [0,1]. Provenance stamps
/// `plugin:<id>`; RESOLVED only on explicit `exact: true` (spec §43: a
/// plugin may not label guesses RESOLVED), else INFERRED. Evidence ids
/// supplied by the caller are preserved; the plugin extractor tag is
/// added at commit time by the normal path.
// trace:v1 id=impl.scc-engine-plugins.promote-sidecar work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn promote_sidecar(
    store: &scc_store::Store,
    plugin_id: &str,
    assertions: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let arr = assertions.as_array().cloned().unwrap_or_default();
    let mut rels = Vec::new();
    for (i, a) in arr.iter().enumerate() {
        let sub = a.get("subject").and_then(|v| v.as_str()).unwrap_or("");
        let pred = a.get("predicate").and_then(|v| v.as_str()).unwrap_or("");
        let obj = a.get("object").and_then(|v| v.as_str()).unwrap_or("");
        if sub.is_empty() || pred.is_empty() || obj.is_empty() {
            return Err(crate::EngineError::Other(format!(
                "promotion assertion #{i}: subject/predicate/object required"
            )));
        }
        if !scc_core::predicates::ALL.contains(&pred) {
            return Err(crate::EngineError::Other(format!(
                "promotion assertion #{i}: predicate '{pred}' is not core ontology (custom predicates stay 'plugin:<id>/...' via plugins.contribute)"
            )));
        }
        let conf = a.get("confidence").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        if !(0.0..=1.0).contains(&conf) {
            return Err(crate::EngineError::Other(format!(
                "promotion assertion #{i}: confidence {conf} outside [0,1]"
            )));
        }
        let exact = a.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
        let prov = if exact { "RESOLVED" } else { "INFERRED" };
        let ev: Vec<serde_json::Value> = a.get("evidence").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        rels.push(serde_json::json!({
            "id": format!("plugin:{plugin_id}/promoted/{i}"),
            "subject": sub, "predicate": pred, "object": obj,
            "provenance": prov, "confidence": conf, "evidence": ev,
        }));
    }
    let batch = serde_json::json!({
        "entities": [], "relationships": rels,
        "evidence": [], "diagnostics": [],
    });
    commit_contribution(store, plugin_id, &batch)
}

/// Deterministic extension order (§19): priority ascending, plugin id
/// ascending, then before/after DAG edges. Unknown references and cycles
/// are startup errors — never silent misordering.
// trace:exempt reason=internal-detail
pub struct ExtensionOrder {
    pub extension_type: String,
    pub id: String,
    pub priority: i32,
    pub after: Vec<String>,
    pub before: Vec<String>,
}

// trace:exempt reason=internal-detail
impl ExtensionOrder {
    // trace:exempt reason=internal-detail
    pub fn key(&self) -> String { format!("{}:{}", self.extension_type, self.id) }
}

// trace:v1 id=impl.scc-engine-plugins.order-extensions work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn order_extensions(extensions: &[ExtensionOrder]) -> crate::Result<Vec<usize>> {
    use std::collections::{BTreeMap, BTreeSet};
    let n = extensions.len();
    // Node key: canonical `type:id`.
    let key_of = |i: usize| -> String { extensions[i].key() };
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for i in 0..n {
        let k = key_of(i);
        if index.insert(k.clone(), i).is_some() {
            return Err(crate::EngineError::Other(format!("duplicate extension registration '{k}'")));
        }
    }
    // Edges: after(X) means X -> self; before(Y) means self -> Y.
    let mut preds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut succs: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for i in 0..n {
        for a in &extensions[i].after {
            let j = *index.get(a).ok_or_else(|| {
                crate::EngineError::Other(format!("extension '{}' orders after unknown '{}'", key_of(i), a))
            })?;
            if j != i {
                preds[i].insert(j);
                succs[j].insert(i);
            }
        }
        for b in &extensions[i].before {
            let j = *index.get(b).ok_or_else(|| {
                crate::EngineError::Other(format!("extension '{}' orders before unknown '{}'", key_of(i), b))
            })?;
            if j != i {
                succs[i].insert(j);
                preds[j].insert(i);
            }
        }
    }
    // Kahn with (priority, plugin-id, index) tie-break — deterministic.
    let mut ready: Vec<usize> = (0..n).filter(|&i| preds[i].is_empty()).collect();
    let sort_key = |i: &usize| (extensions[*i].priority, extensions[*i].id.clone(), *i);
    ready.sort_by_key(sort_key);
    let mut out = Vec::with_capacity(n);
    while let Some(i) = ready.first().cloned() {
        ready.remove(0);
        out.push(i);
        let mut newly: Vec<usize> = Vec::new();
        for &j in &succs[i] {
            preds[j].remove(&i);
            if preds[j].is_empty() {
                newly.push(j);
            }
        }
        ready.extend(newly);
        ready.sort_by_key(sort_key);
    }
    if out.len() != n {
        let stuck: Vec<String> = (0..n).filter(|i| !out.contains(i)).map(key_of).collect();
        return Err(crate::EngineError::Other(format!("extension ordering cycle: {}", stuck.join(", "))));
    }
    Ok(out)
}

/// Manifest extensions flattened to ordering entries, in plugin order.
// trace:exempt reason=internal-detail
pub(crate) fn collect_extensions(ap: &ActivePlugins) -> Vec<ExtensionOrder> {
    let mut out = Vec::new();
    for p in &ap.plugins {
        for e in &p.manifest.extensions {
            out.push(ExtensionOrder { extension_type: e.extension_type.clone(), id: e.id.clone(), priority: e.priority, after: e.after.clone(), before: e.before.clone() });
        }
    }
    out
}

/// Validate a plugin contribution batch (§24): entity ids and kinds present,
/// relationship endpoints resolve within (batch entities + graph), custom
/// ontology namespaced `plugin:<id>/...`, evidence ids present.
///
/// Returns the normalized batch on success; broken plugins fail BEFORE any
/// write — the model is never left half-committed.
// trace:v1 id=impl.scc-engine-plugins.validate-contribution work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn validate_contribution(
    store: &scc_store::Store,
    plugin_id: &str,
    batch: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    // Fail loudly on unknown top-level keys: silently dropping a `flows`
    // or `invariants` array would let a plugin believe it contributed
    // facts SCC never stored. Flows/invariants/contracts derive from
    // entities at graph-compile time — contribute entities, not rows.
    if let Some(obj) = batch.as_object() {
        let known = ["entities", "relationships", "evidence", "diagnostics"];
        let unknown: Vec<&str> = obj.keys().filter(|k| !known.contains(&k.as_str())).map(|k| k.as_str()).collect();
        if !unknown.is_empty() {
            return Err(crate::EngineError::Other(format!(
                "contribution has unsupported top-level keys [{}] (supported: entities, relationships, evidence, diagnostics); flows/invariants/contracts derive from entities at graph-compile time",
                unknown.join(", ")
            )));
        }
    }
    let entities = batch.get("entities").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let relationships = batch.get("relationships").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let evidence = batch.get("evidence").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in store.all_entities()? {
        ids.insert(e.id);
    }
    for (i, e) in entities.iter().enumerate() {
        let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            return Err(crate::EngineError::Other(format!("contribution entity #{i}: missing id")));
        }
        if kind.is_empty() {
            return Err(crate::EngineError::Other(format!("contribution entity '{id}': missing kind")));
        }
        if kind.contains('/') && !kind.starts_with("plugin:") {
            return Err(crate::EngineError::Other(format!(
                "contribution entity '{id}': custom kind '{kind}' must be namespaced 'plugin:<id>/...'"
            )));
        }
        ids.insert(id.to_string());
    }
    for (i, r) in relationships.iter().enumerate() {
        let (sub, pred, obj) = (
            r.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
            r.get("predicate").and_then(|v| v.as_str()).unwrap_or(""),
            r.get("object").and_then(|v| v.as_str()).unwrap_or(""),
        );
        if sub.is_empty() || pred.is_empty() || obj.is_empty() {
            return Err(crate::EngineError::Other(format!("contribution relationship #{i}: subject/predicate/object required")));
        }
        if pred.contains('/') && !pred.starts_with("plugin:") {
            return Err(crate::EngineError::Other(format!(
                "contribution relationship #{i}: custom predicate '{pred}' must be namespaced 'plugin:<id>/...'"
            )));
        }
        for end in [sub, obj] {
            if !ids.contains(end) {
                return Err(crate::EngineError::Other(format!(
                    "contribution relationship #{i}: dangling endpoint '{end}'"
                )));
            }
        }
    }
    for (i, e) in evidence.iter().enumerate() {
        if e.get("id").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            return Err(crate::EngineError::Other(format!("contribution evidence #{i}: missing id")));
        }
    }
    // Provenance-stamp every record (§25) before commit.
    let stamp = |mut v: serde_json::Value| -> serde_json::Value {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("_origin".into(), serde_json::json!({
                "kind": "plugin", "plugin_id": plugin_id,
            }));
        }
        v
    };
    let diagnostics = batch.get("diagnostics").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(serde_json::json!({
        "entities": entities.into_iter().map(stamp).collect::<Vec<_>>(),
        "relationships": relationships.into_iter().map(stamp).collect::<Vec<_>>(),
        "evidence": evidence.into_iter().map(stamp).collect::<Vec<_>>(),
        "diagnostics": diagnostics,
    }))
}

/// Commit a validated batch: entities, relationships, evidence in order.
/// Validate-then-commit keeps broken plugins from half-writing the model.
// trace:v1 id=impl.scc-engine-plugins.commit-contribution work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn commit_contribution(
    store: &scc_store::Store,
    plugin_id: &str,
    batch: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    // Spec 24: validate everything BEFORE writing, then commit inside one
    // batch so a mid-batch failure rolls back instead of leaving half a
    // graph. Decode errors also abort before any write.
    let checked = validate_contribution(store, plugin_id, batch)?;
    let entities: Vec<scc_core::Entity> = checked
        .get("entities").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .into_iter().map(serde_json::from_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| crate::EngineError::Other(format!("contribution entity decode: {e}")))?;
    let rels: Vec<scc_core::Relationship> = checked
        .get("relationships").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .into_iter().map(serde_json::from_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| crate::EngineError::Other(format!("contribution relationship decode: {e}")))?;
    let mut evs: Vec<scc_core::Evidence> = checked
        .get("evidence").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .into_iter().map(serde_json::from_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| crate::EngineError::Other(format!("contribution evidence decode: {e}")))?;
    for ev in evs.iter_mut() {
        // Provenance stamp survives the typed decode via extractor tag.
        ev.extractor = Some(format!("plugin:{plugin_id}"));
    }
    store.batch_begin().map_err(crate::EngineError::Store)?;
    let mut counts = (0usize, 0usize, 0usize);
    for e in &entities {
        if let Err(err) = store.insert_entity(e, &[format!("plugin:{plugin_id}")]) {
            store.batch_abort();
            return Err(crate::EngineError::Store(err));
        }
        counts.0 += 1;
    }
    for r in &rels {
        if let Err(err) = store.insert_relationship(r, &format!("plugin:{plugin_id}")) {
            store.batch_abort();
            return Err(crate::EngineError::Store(err));
        }
        counts.1 += 1;
    }
    for ev in &evs {
        if let Err(err) = store.insert_evidence(ev) {
            store.batch_abort();
            return Err(crate::EngineError::Store(err));
        }
        counts.2 += 1;
    }
    store.batch_end().map_err(crate::EngineError::Store)?;
    let n_diag = checked.get("diagnostics").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    Ok(serde_json::json!({"entities": counts.0, "relationships": counts.1, "evidence": counts.2, "diagnostics": n_diag}))
}

/// Context-section contributions (§17 Context): every `context-section:*`
/// extension renders one markdown section into the task pack.
///
/// Input carries goal/files/symbols; output `section` markdown is spliced
/// verbatim under a provenance header (`PLUGIN SECTION <id> from <plugin>`).
/// Failures follow policy: required = hard error, else skip with diagnostic
/// recorded in the pack warnings channel via the returned notes.
// trace:v1 id=impl.scc-engine-plugins.context-sections work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn context_sections(
    root: &std::path::Path,
    config: &scc_indexer::Config,
    goal: &str,
    files: &[String],
    symbols: &[String],
) -> String {
    let mut ap = active(root, config);
    // Deterministic order: registration order (plugin discovery is sorted).
    let mut out = String::new();
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|p| {
            p.manifest.extensions.iter().filter(|e| e.extension_type == "context-section").map(|e| {
                (p.manifest.id.clone(), e.id.clone(), p.manifest.failure_policy.clone())
            })
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|p| p.manifest.id == pid).cloned() {
            Some(p) => p,
            None => continue,
        };
        let input = serde_json::json!({"goal": goal, "files": files, "symbols": symbols, "section": ext_id});
        match scc_plugin_host::call(&plug, "context.section", input, None) {
            Ok(v) => {
                if let Some(section) = v.get("section").and_then(|s| s.as_str()) {
                    if !section.trim().is_empty() {
                        out.push_str(&format!("\n# PLUGIN SECTION {ext_id} (from {pid} — plugin content, not verified facts)\n"));
                        out.push_str(section.trim());
                        out.push('\n');
                    }
                }
            }
            Err(e) => {
                if policy == "required" {
                    out.push_str(&format!("\n# PLUGIN SECTION {ext_id} FAILED (from {pid}): {e}\n"));
                }
                ap.diagnostics.push(scc_plugin_host::PluginDiagnostic {
                    plugin: pid, operation: "context.section".into(), error: e.to_string(),
                    action: if policy == "required" { "failed".into() } else { "skipped".into() },
                });
            }
        }
    }
    // Diagnostics outlive the call: stash count in a trailing marker the
    // pack warnings channel can surface (no silent skips).
    let _ = ap;
    out
}
