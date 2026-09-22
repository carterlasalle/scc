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
