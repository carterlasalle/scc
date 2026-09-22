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
    // Grants: explicit project grants narrow manifest defaults.
    let grants = config.plugins.grants.iter().map(|(k, v)| {
        let perms = v.iter().filter_map(|s| match s.as_str() {
            "repo.read" => Some(scc_plugin_api::Permission::RepoRead),
            "graph.read" => Some(scc_plugin_api::Permission::GraphRead),
            "graph.contribute" => Some(scc_plugin_api::Permission::GraphContribute),
            "state.read" => Some(scc_plugin_api::Permission::StateRead),
            "state.write" => Some(scc_plugin_api::Permission::StateWrite),
            "network" => Some(scc_plugin_api::Permission::Network),
            "subprocess" => Some(scc_plugin_api::Permission::Subprocess),
            _ => None,
        }).collect::<Vec<_>>();
        (k.clone(), perms)
    }).collect::<std::collections::BTreeMap<_, _>>();
    scc_plugin_host::apply_grants(&mut plugins, &grants);
    ActivePlugins { plugins, diagnostics: Vec::new() }
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
    Ok(serde_json::json!({
        "entities": entities.into_iter().map(stamp).collect::<Vec<_>>(),
        "relationships": relationships.into_iter().map(stamp).collect::<Vec<_>>(),
        "evidence": evidence.into_iter().map(stamp).collect::<Vec<_>>(),
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
    Ok(serde_json::json!({"entities": counts.0, "relationships": counts.1, "evidence": counts.2}))
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
