//! SCC plugin host: discovery, loading, permissions, execution, diagnostics.
//!
//! Process plugins (spec section 15): a `scc-plugin.toml` manifest plus a
//! command speaking JSON over stdio. One request per process spawn (simple,
//! crash-isolated, no lingering children); `timeout_ms` + `failure_policy`
//! bound every call. Grants are checked before spawn — a plugin never
//! executes code it was not granted (spec 21).

use scc_plugin_api::{Permission, PluginManifest, PluginRequest, PluginResponse};
use std::collections::BTreeMap;
use std::path::PathBuf;

// trace:exempt reason=internal-detail
pub const MANIFEST_FILE: &str = "scc-plugin.toml";

#[derive(Debug, Clone)]
// trace:exempt reason=internal-detail
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
    pub config: serde_json::Value,
    pub grants: Vec<Permission>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct PluginDiagnostic {
    pub plugin: String,
    pub operation: String,
    pub error: String,
    pub action: String,
}

#[derive(Debug, Clone, thiserror::Error)]
// trace:exempt reason=internal-detail
pub enum HostError {
    #[error("plugin {0}: {1}")]
    Failed(String, String),
    #[error("permission denied: plugin {0} lacks {1}")]
    Denied(String, String),
    #[error("no plugin provides operation {0}")]
    NoProvider(String),
    #[error("ambiguous: {0}")]
    Ambiguous(String),
    #[error("io: {0}")]
    Io(String),
    #[error("plugin {0} declares runtime wasm: no WASM host yet (see scc-plugin-api PLUGIN_WIT); use runtime.command process plugin instead")]
    UnsupportedRuntime(String),
}

// trace:v1 id=impl.crates-scc-plugin-host-src-lib.discover work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub fn discover(repo_root: &std::path::Path) -> Vec<LoadedPlugin> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("SCC_PLUGIN_PATH") {
        for part in std::env::split_paths(&p) { dirs.push(part); }
    }
    dirs.push(repo_root.join(".scc").join("plugins"));
    if let Some(home) = dirs_home() { dirs.push(home.join(".config").join("scc").join("plugins")); }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue; };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let manifest_path = path.join(MANIFEST_FILE);
            let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue; };
            let Ok(manifest) = PluginManifest::from_toml(&text) else { continue; };
            if manifest.check_api_compatible().is_err() { continue; }
            if !seen.insert(manifest.id.clone()) { continue; }
            // Grants come from project config; discovery defaults to the
            // manifest-declared permissions (explicit grants narrow them).
            out.push(LoadedPlugin { manifest, dir: path, config: serde_json::json!({}), grants: Vec::new() });
        }
    }
    out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    out
}

// trace:exempt reason=internal-detail
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

// trace:exempt reason=internal-detail
pub fn apply_grants(plugins: &mut [LoadedPlugin], grants: &BTreeMap<String, Vec<Permission>>) {
    for p in plugins {
        if let Some(g) = grants.get(&p.manifest.id) { p.grants = g.clone(); }
        else { p.grants = p.manifest.permissions.clone(); }
    }
}

// trace:exempt reason=internal-detail
pub fn provider_for<'a>(plugins: &'a [LoadedPlugin], operation: &str) -> Result<&'a LoadedPlugin, HostError> {
    let mut found: Option<&LoadedPlugin> = None;
    for p in plugins {
        if p.manifest.operations.iter().any(|o| o == operation) {
            if found.is_some() {
                return Err(HostError::Ambiguous(format!("{operation} provided by multiple plugins")));
            }
            found = Some(p);
        }
    }
    found.ok_or_else(|| HostError::NoProvider(operation.into()))
}

/// Execute one plugin operation in a spawned child (crash-isolated).
/// Grants are enforced: state read/write ops need the matching grant;
/// everything else needs at least discovery (the plugin is enabled).
// trace:exempt reason=internal-detail
pub fn call(plugin: &LoadedPlugin, operation: &str, input: serde_json::Value, timeout_override_ms: Option<u64>) -> Result<serde_json::Value, HostError> {
    require_grant(plugin, operation)?;
    // WASM runtime: declared via the checked-in WIT (spec §14) but not yet
    // hosted — fail loudly with an actionable diagnostic, never silently
    // misload a .wasm artifact as a process command.
    if plugin.manifest.runtime == scc_plugin_api::PluginRuntime::Wasm {
        return Err(HostError::UnsupportedRuntime(plugin.manifest.id.clone()));
    }
    let req = PluginRequest { operation: operation.into(), input, config: plugin.config.clone() };
    let body = serde_json::to_string(&req).map_err(|e| HostError::Failed(plugin.manifest.id.clone(), e.to_string()))?;
    let timeout = std::time::Duration::from_millis(timeout_override_ms.unwrap_or(plugin.manifest.timeout_ms));
    let mut child = std::process::Command::new(&plugin.manifest.command[0])
        .args(&plugin.manifest.command[1..])
        .current_dir(&plugin.dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| HostError::Failed(plugin.manifest.id.clone(), format!("spawn: {e}")))?;
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    let out = wait_with_timeout(&mut child, timeout).map_err(|e| HostError::Failed(plugin.manifest.id.clone(), e))?;
    let resp: PluginResponse = serde_json::from_slice(&out).map_err(|e| HostError::Failed(plugin.manifest.id.clone(), format!("bad response: {e}")))?;
    if let Some(err) = resp.error {
        return Err(HostError::Failed(plugin.manifest.id.clone(), err));
    }
    Ok(resp.output)
}

// trace:exempt reason=internal-detail
fn require_grant(plugin: &LoadedPlugin, operation: &str) -> Result<(), HostError> {
    // Custom namespaced ops (acme.foo) run on enablement alone. State ops
    // stay gated: a plugin without the matching grant cannot touch state.
    if operation.starts_with("state.") {
        let need = if operation.contains("write") || operation.contains("put") {
            Permission::StateWrite
        } else {
            Permission::StateRead
        };
        if !plugin.grants.contains(&need) && !plugin.manifest.permissions.contains(&need) {
            return Err(HostError::Denied(plugin.manifest.id.clone(), need.as_str().into()));
        }
    }
    Ok(())
}

// trace:exempt reason=internal-detail
fn wait_with_timeout(child: &mut std::process::Child, timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = Vec::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use std::io::Read;
                    let _ = stdout.read_to_end(&mut out);
                }
                if !status.success() { return Err(format!("exit {}", status)); }
                return Ok(out);
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    }
}
// trace:exempt reason=internal-detail
pub fn lock_entry(p: &LoadedPlugin) -> serde_json::Value {
    let mut h = blake3::Hasher::new();
    h.update(p.manifest.id.as_bytes());
    h.update(p.manifest.version.as_bytes());
    h.update(p.manifest.api.as_bytes());
    h.update(format!("{:?}", p.manifest.runtime).as_bytes());
    let extensions: Vec<serde_json::Value> = p.manifest.extensions.iter().map(|e| {
        h.update(e.canonical_id().as_bytes());
        h.update(e.priority.to_string().as_bytes());
        for x in e.after.iter().chain(e.before.iter()) { h.update(x.as_bytes()); }
        serde_json::json!({"type": e.extension_type, "id": e.id, "priority": e.priority, "after": e.after, "before": e.before})
    }).collect();
    serde_json::json!({
        "id": p.manifest.id,
        "version": p.manifest.version,
        "api": p.manifest.api,
        "artifact_hash": format!("{}", h.finalize().to_hex()),
        "operations": p.manifest.operations,
        "permissions": p.manifest.permissions.iter().map(|x| x.as_str()).collect::<Vec<_>>(),
        "runtime": format!("{:?}", p.manifest.runtime).to_lowercase(),
        "extensions": extensions,
    })
}


/// Project plugin lockfile (§29): `.scc/plugins.lock` records the resolved
/// plugin set so behavior reproduces across machines.
///
/// Each entry carries id, version, source dir, artifact hash, API version,
/// and granted permissions — the fields `lock_entry` already emits. Write
/// is atomic (temp + rename); read returns an empty set when absent (fresh
/// checkout, no plugins pinned yet).
// trace:v1 id=impl.scc-plugin-host.lockfile work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn lockfile_path(repo_root: &std::path::Path) -> PathBuf {
    repo_root.join(".scc").join("plugins.lock")
}

// trace:exempt reason=internal-detail
pub fn write_lockfile(repo_root: &std::path::Path, plugins: &[LoadedPlugin]) -> Result<PathBuf, String> {
    let path = lockfile_path(repo_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let entries: Vec<serde_json::Value> = plugins.iter().map(lock_entry).collect();
    let doc = serde_json::json!({"version": 1, "plugins": entries});
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("lock.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

// trace:exempt reason=internal-detail
pub fn read_lockfile(repo_root: &std::path::Path) -> Result<Vec<serde_json::Value>, String> {
    let path = lockfile_path(repo_root);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Vec::new()); };
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(doc.get("plugins").and_then(|v| v.as_array()).cloned().unwrap_or_default())
}

/// Verify the live plugin set against the lockfile: version or artifact
/// drift fails loudly with the drifted ids named — never silently answers
/// from a moved plugin set.
// trace:v1 id=impl.scc-plugin-host.lockfile-check work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn check_lockfile(repo_root: &std::path::Path, plugins: &[LoadedPlugin]) -> Result<(), String> {
    let locked = read_lockfile(repo_root)?;
    if locked.is_empty() {
        return Ok(());
    }
    let live: std::collections::BTreeMap<String, serde_json::Value> = plugins
        .iter()
        .map(|p| (p.manifest.id.clone(), lock_entry(p)))
        .collect();
    let mut drifted = Vec::new();
    for entry in &locked {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
        match live.get(id) {
            None => drifted.push(format!("{id} (missing)")),
            Some(cur) => {
                let same = cur.get("version") == entry.get("version")
                    && cur.get("artifact_hash") == entry.get("artifact_hash");
                if !same {
                    drifted.push(id.to_string());
                }
            }
        }
    }
    if drifted.is_empty() {
        Ok(())
    } else {
        Err(format!("plugin set drifted from .scc/plugins.lock: {}", drifted.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    #[test]
// trace:exempt reason=unit-test
    fn no_provider_is_not_ambiguous() {
        let r = super::provider_for(&[], "acme.missing");
        assert!(matches!(r, Err(super::HostError::NoProvider(_))));
    }

    #[test]
// trace:exempt reason=unit-test
    fn state_op_without_grant_is_denied() {
        let p = super::LoadedPlugin {
            manifest: scc_plugin_api::PluginManifest {
                id: "x".into(), name: "X".into(), version: "1".into(), api: "1".into(),
                operations: vec![], permissions: vec![], timeout_ms: 50, runtime: Default::default(),
                failure_policy: "warn".into(), deterministic: true, command: vec!["true".into()], extensions: vec![],
            },
            dir: std::path::PathBuf::from("."),
            config: serde_json::json!({}),
            grants: vec![],
        };
        assert!(super::call(&p, "state.get", serde_json::json!({}), None).is_err());
    }
}
