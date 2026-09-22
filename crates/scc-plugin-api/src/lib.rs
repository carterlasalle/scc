//! Stable SCC plugin contracts (spec sections 15-16, 21, 31).
//!
//! Plugins depend on THIS crate only — never on `scc-engine` or `scc-cli`.
//! Manifests are `scc-plugin.toml`; the process-plugin wire protocol is
//! plain JSON over stdio using [`PluginRequest`]/[`PluginResponse`].

use serde::{Deserialize, Serialize};

// trace:exempt reason=internal-detail
pub const PLUGIN_API_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
pub enum Permission {
    RepoRead,
    GraphRead,
    GraphContribute,
    StateRead,
    StateWrite,
    Network,
    Subprocess,
}

// trace:exempt reason=internal-detail
impl Permission {
// trace:exempt reason=internal-detail
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::RepoRead => "repo.read",
            Permission::GraphRead => "graph.read",
            Permission::GraphContribute => "graph.contribute",
            Permission::StateRead => "state.read",
            Permission::StateWrite => "state.write",
            Permission::Network => "network",
            Permission::Subprocess => "subprocess",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:v1 id=impl.crates-scc-plugin-api-src-lib.plugin-manifest work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "default_api")]
    pub api: String,
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_policy")]
    pub failure_policy: String,
    #[serde(default = "default_true")]
    pub deterministic: bool,
    #[serde(skip)]
    pub command: Vec<String>,
}

// trace:exempt reason=internal-detail
fn default_api() -> String { "1".into() }
// trace:exempt reason=internal-detail
fn default_timeout() -> u64 { 5000 }
// trace:exempt reason=internal-detail
fn default_policy() -> String { "warn".into() }
// trace:exempt reason=internal-detail
fn default_true() -> bool { true }

// trace:exempt reason=internal-detail
impl PluginManifest {
// trace:exempt reason=internal-detail
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut m = PluginManifest {
            id: String::new(), name: String::new(), version: String::new(),
            api: default_api(), operations: Vec::new(), permissions: Vec::new(),
            timeout_ms: default_timeout(), failure_policy: default_policy(),
            deterministic: true, command: Vec::new(),
        };
        let mut section = String::new();
        for (ln, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len()-1].trim().to_string();
                continue;
            }
            let (k, v) = line.split_once('=').ok_or(format!("line {}: expected key = value", ln + 1))?;
            let (k, v) = (k.trim(), v.trim());
            let unq = |s: &str| s.trim_matches('"').trim_matches('\'').to_string();
            let flag = v == "true";
            match (section.as_str(), k) {
                ("plugin", "id") => m.id = unq(v),
                ("plugin", "name") => m.name = unq(v),
                ("plugin", "version") => m.version = unq(v),
                ("plugin", "api") => m.api = unq(v),
                ("plugin", "operations") => m.operations = parse_str_array(v)?,
                ("plugin", "timeout_ms") => m.timeout_ms = v.parse().unwrap_or(5000),
                ("plugin", "failure_policy") => m.failure_policy = unq(v),
                ("plugin", "deterministic") => m.deterministic = flag,
                ("runtime", "command") => m.command = parse_str_array(v)?,
                ("permissions", key) => {
                    let perm = match key {
                        "repo_read" => Permission::RepoRead,
                        "graph_read" => Permission::GraphRead,
                        "graph_contribute" => Permission::GraphContribute,
                        "state_read" => Permission::StateRead,
                        "state_write" => Permission::StateWrite,
                        "network" => Permission::Network,
                        "subprocess" => Permission::Subprocess,
                        _ => continue,
                    };
                    if flag { m.permissions.push(perm); }
                }
                _ => {}
            }
        }
        if m.id.is_empty() { return Err("missing plugin.id".into()); }
        if m.command.is_empty() { return Err("missing runtime.command".into()); }
        Ok(m)
    }

    /// Compatibility check (spec 41): never "try it and see whether it crashes".
// trace:exempt reason=internal-detail
    pub fn check_api_compatible(&self) -> Result<(), String> {
        let major = self.api.split('.').next().unwrap_or("");
        if major == "1" || self.api == "1" { Ok(()) } else {
            Err(format!("plugin '{}' requires plugin API {}, host provides {}", self.id, self.api, PLUGIN_API_VERSION))
        }
    }
}

// trace:exempt reason=internal-detail
fn parse_str_array(s: &str) -> Result<Vec<String>, String> {
    let s = s.trim();
    if !s.starts_with('[') || !s.ends_with(']') { return Err(format!("expected string array, got {s:?}")); }
    let mut out = Vec::new();
    for part in s[1..s.len()-1].split(',') {
        let part = part.trim();
        if part.is_empty() { continue; }
        out.push(part.trim_matches('"').trim_matches('\'').to_string());
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct PluginRequest {
    pub operation: String,
    pub input: serde_json::Value,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct PluginResponse {
    #[serde(default)]
    pub output: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    #[test]
// trace:exempt reason=unit-test
    fn manifest_parses() {
        let m = super::PluginManifest::from_toml("[plugin]\nid = \"acme.demo\"\noperations = [\"acme.echo\"]\n[runtime]\ncommand = [\"python3\", \"p.py\"]\n[permissions]\nrepo_read = true\n").unwrap();
        assert_eq!(m.id, "acme.demo");
        assert_eq!(m.command, vec!["python3", "p.py"]);
        assert!(m.check_api_compatible().is_ok());
    }

    #[test]
// trace:exempt reason=unit-test
    fn incompatible_api_rejected() {
        let m = super::PluginManifest::from_toml("[plugin]\nid = \"x\"\napi = \"2\"\n[runtime]\ncommand = [\"a\"]\n").unwrap();
        assert!(m.check_api_compatible().is_err());
    }
}
