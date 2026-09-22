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

    /// Parse a project-config grant name. Unknown names are an `Err` —
    /// callers must surface it (diagnostic or hard error), never silently
    /// narrow the grant set (a typo'd permission must not read as "deny").
    // trace:exempt reason=internal-detail
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "repo.read" => Ok(Permission::RepoRead),
            "graph.read" => Ok(Permission::GraphRead),
            "graph.contribute" => Ok(Permission::GraphContribute),
            "state.read" => Ok(Permission::StateRead),
            "state.write" => Ok(Permission::StateWrite),
            "network" => Ok(Permission::Network),
            "subprocess" => Ok(Permission::Subprocess),
            other => Err(format!(
                "unknown permission '{other}' (known: repo.read, graph.read, graph.contribute, state.read, state.write, network, subprocess)"
            )),
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
    #[serde(default = "default_runtime")]
    pub runtime: PluginRuntime,
    /// Declared extension registrations (spec §17): (extension-type, id,
    /// priority, after, before). Process plugins declare these in
    /// `[extensions] "rank-feature:acme.id" = {priority=10, after=[...]}`.
    #[serde(default)]
    pub extensions: Vec<ExtensionRegistration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ExtensionRegistration {
    #[serde(rename = "type")]
    pub extension_type: String,
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub before: Vec<String>,
}

// trace:exempt reason=internal-detail
impl ExtensionRegistration {
    /// Canonical id `type:id` (e.g. `rank-feature:acme.risk`).
// trace:exempt reason=internal-detail
    pub fn canonical_id(&self) -> String { format!("{}:{}", self.extension_type, self.id) }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// trace:exempt reason=internal-detail
pub enum PluginRuntime {
    /// External process over stdio (spec §15). The only supported runtime
    /// today; `command` carries the argv.
    #[default]
    Process,
    /// WASM Component Model (spec §14): declared via the checked-in WIT.
    /// Loading is a loud `UnsupportedRuntime` diagnostic until the wasmtime
    /// host lands — never silent misloading.
    Wasm,
}

// trace:exempt reason=internal-detail
fn default_runtime() -> PluginRuntime { PluginRuntime::Process }

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
            deterministic: true, command: Vec::new(), runtime: PluginRuntime::Process, extensions: Vec::new(),
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
                ("runtime", "type") => m.runtime = match unq(v).as_str() {
                    "wasm" => PluginRuntime::Wasm,
                    _ => PluginRuntime::Process,
                },
                ("extensions", k) => m.extensions.push(parse_extension(k, v)?),
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

// trace:v1 id=impl.crates-scc-plugin-api-src-lib.plugin-manifest-extension work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
fn parse_extension(key: &str, value: &str) -> Result<ExtensionRegistration, String> {
    // Key: `"rank-feature:acme.id"` (quotes stripped). Value: inline table
    // `{priority=10, after=[...], before=[...]}`; bare values default.
    let key = key.trim_matches('"').trim_matches('\'');
    let (extension_type, id) = key.split_once(':').ok_or(format!("bad extension key {key:?} (want \"type:id\")"))?;
    let mut reg = ExtensionRegistration {
        extension_type: extension_type.trim().into(),
        id: id.trim().into(),
        ..Default::default()
    };
    let body = value.trim().trim_matches(|c| c == '{' || c == '}');
    for part in body.split(',') {
        let part = part.trim();
        if part.is_empty() { continue; }
        let (k, v) = part.split_once('=').ok_or(format!("bad extension field {part:?}"))?;
        match k.trim() {
            "priority" => reg.priority = v.trim().parse().unwrap_or(0),
            "after" => reg.after = parse_str_array(v)?,
            "before" => reg.before = parse_str_array(v)?,
            _ => {}
        }
    }
    if reg.extension_type.is_empty() || reg.id.is_empty() {
        return Err(format!("bad extension key {key:?}"));
    }
    Ok(reg)
}

/// WIT interface definition for the WASM Component Model host (spec §14).
/// Checked in as the versioned ABI contract: any runtime implementing this
/// world (wasmtime-based or otherwise) hosts `scc-plugin.wit` plugins.
/// The JSON shapes (`PluginRequest`/`PluginResponse`) are identical to the
/// process-plugin wire protocol, so a plugin written against these schemas
/// runs on either runtime unchanged.
// trace:exempt reason=internal-detail
pub const PLUGIN_WIT: &str = include_str!("plugin.wit");

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

    #[test]
// trace:exempt reason=unit-test
    fn extensions_parse_with_ordering() {
        let m = super::PluginManifest::from_toml("[plugin]\nid = \"x\"\n[runtime]\ncommand = [\"a\"]\n[extensions]\n\"rank-feature:acme.risk\" = {priority=10, after=[\"core.lex\"], before=[]}\n").unwrap();
        assert_eq!(m.extensions.len(), 1);
        let e = &m.extensions[0];
        assert_eq!(e.canonical_id(), "rank-feature:acme.risk");
        assert_eq!(e.priority, 10);
        assert_eq!(e.after, vec!["core.lex"]);
    }

    #[test]
// trace:exempt reason=unit-test
    fn wit_contract_declares_plugin_world() {
        // The checked-in WIT is the versioned ABI: world, host imports,
        // and the three plugin exports must all be present.
        assert!(super::PLUGIN_WIT.contains("world scc-plugin"));
        assert!(super::PLUGIN_WIT.contains("invoke-hook"));
        assert!(super::PLUGIN_WIT.contains("manifest: func()"));
        assert!(super::PLUGIN_WIT.contains("register: func()"));
    }
}

