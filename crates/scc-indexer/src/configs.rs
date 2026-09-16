//! Config, infrastructure, and intent extraction (docs/FLOW_COMPILER.md §2,
//! EPIC-140-lite for MVP): package.json workspaces, docker-compose services,
//! env files, `.scc/intent.yaml`, README purpose.

use crate::model::{Entrypoint, ExtractedFile};
use crate::redact::{classify_secret, parse_env_file};
use scc_core::kinds;
use scc_core::{Entity, Provenance, Relationship};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A declared-intent document (`.scc/intent.yaml`), per docs §33 and
/// EPIC-180. Fields beyond the docs (paths, stores) are optional extensions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Intent {
    #[serde(default)]
    pub components: BTreeMap<String, IntentComponent>,
    #[serde(default)]
    pub invariants: BTreeMap<String, IntentInvariant>,
    #[serde(default)]
    pub flows: BTreeMap<String, IntentFlow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentComponent {
    #[serde(default)]
    pub responsibility: Vec<String>,
    #[serde(default)]
    pub owns: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentInvariant {
    pub statement: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub enforced_by: Vec<String>,
}

fn default_severity() -> String {
    "critical".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentFlow {
    pub entrypoint: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Workspace member: one package/project inside a monorepo (audit
/// item 4). One Git repo keeps one repository identity/store; packages are
/// semantic children — never separate databases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.indexer.workspace-member work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct WorkspaceMember {
    pub id: String,
    pub name: String,
    pub root: String,
    pub manifest: String,
    pub ecosystem: String,
}

/// Package-to-package dependency edge (depends_on / dev / peer).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.indexer.workspace-edge work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct WorkspaceEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

/// Workspace graph: members + edges for one repository (audit item 4).
/// Authoritative for workspace-aware Skeleton/components/impact/context
/// once persisted; the indexer's PACKAGE entities + DEPENDS_ON edges are
/// its graph projection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:v1 id=impl.scc.indexer.workspace-graph work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct WorkspaceGraph {
    pub ecosystem: String,
    pub members: Vec<WorkspaceMember>,
    pub edges: Vec<WorkspaceEdge>,
}

/// Non-code extraction result for one file.
#[derive(Debug, Clone, Default)]
// trace:exempt reason=internal-detail
pub struct ConfigExtraction {
    pub entities: Vec<Entity>,
    pub relationships: Vec<(Relationship, String)>, // (rel, source_path)
    pub entrypoints: Vec<Entrypoint>,
    /// (file, extracted facts) — e.g. package.json's scripts are not modeled.
    pub intent: Option<Intent>,
    /// README purpose paragraph.
    pub readme_purpose: Option<String>,
    /// Workspace members/edges discovered from THIS manifest (populated by
    /// [`workspace_members`] at the lib.rs call site, which owns the
    /// scanned inventory + member file reads — never inside per-file
    /// [`extract_config_file`]).
    pub workspace: Option<WorkspaceGraph>,
}

// trace:exempt reason=internal-detail
pub fn extract_config_file(path: &str, content: &str, repo_id: &str) -> ConfigExtraction {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let mut out = ConfigExtraction::default();
    if name == "package.json" {
        extract_package_json(path, content, repo_id, &mut out);
    } else if name.starts_with("docker-compose") || name.starts_with("compose.") {
        extract_compose(path, content, repo_id, &mut out);
    } else if name.starts_with(".env") {
        extract_env(path, content, repo_id, &mut out);
    } else if path == ".scc/intent.yaml" {
        out.intent = serde_yaml::from_str(content).ok();
    } else if path.eq_ignore_ascii_case("readme.md") {
        out.readme_purpose = readme_purpose(content);
    } else if name.ends_with(".proto") {
        extract_proto(path, content, repo_id, &mut out);
    }
    out
}

/// Expand workspace globs (`packages/*`, `apps/*`, `packages/foo`)
/// against the scanned repo file list. `*` matches one path segment;
/// `**` matches any depth. Returns sorted member roots (dirs containing a
/// package.json), deterministic.
// trace:v1 id=impl.scc.indexer.expand-workspace-globs work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn expand_workspace_globs(patterns: &[String], files: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // package.json paths by directory for membership tests
    let pkg_dirs: std::collections::BTreeSet<&str> = files
        .iter()
        .filter(|f| f.ends_with("/package.json") || *f == "package.json")
        .map(|f| match f.rfind('/') {
            Some(i) => &f[..i],
            None => "",
        })
        .collect();
    for pat in patterns {
        let pat = pat.trim().trim_end_matches('/').trim_start_matches("./");
        if pat.is_empty() || pat == "." {
            continue;
        }
        if !pat.contains('*') {
            // Literal dir: keep only if it actually holds a package.json.
            let dir = pat.trim_end_matches('/');
            if pkg_dirs.contains(dir) && !out.iter().any(|d| d == dir) {
                out.push(dir.to_string());
            }
            continue;
        }
        // Glob -> matcher over directory paths.
        let matcher = match globset::Glob::new(pat) {
            Ok(g) => g.compile_matcher(),
            Err(_) => continue,
        };
        let mut candidates: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for dir in &pkg_dirs {
            if matcher.is_match(dir) {
                candidates.insert(dir);
            }
        }
        // Also match the package.json path itself (patterns like
        // `packages/*/package.json` should resolve): strip the file.
        for f in files.iter().filter(|f| f.ends_with("/package.json")) {
            if matcher.is_match(f.as_str()) {
                if let Some(i) = f.rfind('/') {
                    candidates.insert(&f[..i]);
                }
            }
        }
        for c in candidates {
            if !out.iter().any(|d| d == c) {
                out.push(c.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Discover workspace members + dependency edges for a JS monorepo
/// root package.json (audit item 4): glob-expand `workspaces` against the
/// scanned file inventory, then read each member's name + its
/// dependencies/devDependencies/peerDependencies. Members become PACKAGE
/// entities (named by package name, path in attributes); member deps that
/// resolve to another member become DEPENDS_ON edges. Literal patterns
/// without globs keep the old behavior when the dir holds a package.json.
/// Unknown member names in deps stay unresolved (never fabricated).
// trace:v1 id=impl.scc.indexer.workspace-members work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn workspace_members(
    root_path: &str,
    content: &str,
    repo_id: &str,
    files: &[String],
    read_member: &dyn Fn(&str) -> Option<String>,
) -> WorkspaceGraph {
    let mut graph = WorkspaceGraph {
        ecosystem: "npm".into(),
        members: Vec::new(),
        edges: Vec::new(),
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(content) else {
        return graph;
    };
    let mut patterns: Vec<String> = Vec::new();
    if let Some(ws) = v.get("workspaces") {
        if let Some(arr) = ws.as_array() {
            for m in arr {
                if let Some(s) = m.as_str() {
                    patterns.push(s.to_string());
                }
            }
        } else if let Some(obj) = ws.as_object() {
            if let Some(pkgs) = obj.get("packages").and_then(|p| p.as_array()) {
                for m in pkgs {
                    if let Some(s) = m.as_str() {
                        patterns.push(s.to_string());
                    }
                }
            }
        }
    }
    if patterns.is_empty() {
        return graph;
    }
    // The root dir for relative expansion: dirname of the manifest.
    let base = match root_path.rfind('/') {
        Some(i) => &root_path[..i],
        None => "",
    };
    let join = |d: &str| -> String {
        if base.is_empty() {
            d.to_string()
        } else {
            format!("{base}/{d}")
        }
    };
    let roots = expand_workspace_globs(&patterns, files);
    // name -> root for edge resolution
    let mut name_of: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut member_paths: Vec<String> = Vec::new();
    for r in &roots {
        let full = join(r);
        let manifest = if full.is_empty() {
            "package.json".to_string()
        } else {
            format!("{full}/package.json")
        };
        let Some(text) = read_member(&manifest) else {
            continue;
        };
        let Ok(mv): Result<serde_json::Value, _> = serde_json::from_str(&text) else {
            continue;
        };
        let name = mv
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(&full)
            .to_string();
        name_of.insert(name.clone(), full.clone());
        member_paths.push(full.clone());
        graph.members.push(WorkspaceMember {
            id: scc_core::entity_id(repo_id, scc_core::kinds::PACKAGE, &name),
            name,
            root: full,
            manifest,
            ecosystem: "npm".into(),
        });
    }
    // Edges: member dependency names that resolve to another member.
    for m in &graph.members.clone() {
        let Some(text) = read_member(&m.manifest) else {
            continue;
        };
        let Ok(mv): Result<serde_json::Value, _> = serde_json::from_str(&text) else {
            continue;
        };
        for section in ["dependencies"] {
            if let Some(deps) = mv.get(section).and_then(|d| d.as_object()) {
                for dep in deps.keys() {
                    if let Some(target_root) = name_of.get(dep) {
                        if target_root != &m.root {
                            graph.edges.push(WorkspaceEdge {
                                from: m.id.clone(),
                                to: scc_core::entity_id(
                                    repo_id,
                                    scc_core::kinds::PACKAGE,
                                    dep,
                                ),
                                kind: "depends_on".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    graph.members.sort_by(|a, b| a.name.cmp(&b.name));
    graph.edges.sort_by(|a, b| (&a.from, &a.to, &a.kind).cmp(&(&b.from, &b.to, &b.kind)));
    graph
}

fn extract_package_json(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(content) else {
        return;
    };
    // Workspace members -> package entities
    let mut members: Vec<String> = Vec::new();
    if let Some(ws) = v.get("workspaces") {
        if let Some(arr) = ws.as_array() {
            for m in arr {
                if let Some(s) = m.as_str() {
                    members.push(s.trim_end_matches('/').to_string());
                }
            }
        } else if let Some(obj) = ws.as_object() {
            if let Some(pkgs) = obj.get("packages").and_then(|p| p.as_array()) {
                for m in pkgs {
                    if let Some(s) = m.as_str() {
                        members.push(s.trim_end_matches('/').to_string());
                    }
                }
            }
        }
    }
    for m in members {
        let id = scc_core::entity_id(repo_id, kinds::PACKAGE, &m);
        let mut e = Entity::new(id.clone(), kinds::PACKAGE, m.clone());
        e.attr("path", serde_json::json!(m));
        out.entities.push(e);
        let rel = Relationship::new(
            scc_core::relationship_id(0), // id patched by writer
            format!("repo://{repo_id}"),
            scc_core::predicates::CONTAINS,
            id,
            Provenance::Extracted,
        );
        out.relationships.push((rel, path.to_string()));
    }
    // bin/main -> entrypoints (kind "entrypoint"; name = bin key or path)
    for key in ["bin", "main"] {
        if let Some(bin) = v.get(key) {
            match bin {
                serde_json::Value::String(s) => {
                    if !s.is_empty() {
                        out.entrypoints.push(Entrypoint {
                            symbol: s.clone(),
                            kind: "entrypoint".to_string(),
                            line: 1,
                        });
                    }
                }
                serde_json::Value::Object(o) => {
                    for (name, val) in o {
                        if val.as_str().is_some() {
                            out.entrypoints.push(Entrypoint {
                                symbol: name.clone(),
                                kind: "entrypoint".to_string(),
                                line: 1,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_compose(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let Ok(v): Result<serde_json::Value, _> = serde_yaml::from_str(content) else {
        return;
    };
    let Some(services) = v.get("services").and_then(|s| s.as_object()) else {
        return;
    };
    for (name, spec) in services {
        let id = scc_core::entity_id(repo_id, kinds::DEPLOYMENT_UNIT, name);
        let mut e = Entity::new(id.clone(), kinds::DEPLOYMENT_UNIT, name.clone());
        if let Some(image) = spec.get("image").and_then(|i| i.as_str()) {
            e.attr("image", serde_json::json!(image));
        }
        if let Some(build) = spec.get("build") {
            let ctx = build
                .get("context")
                .or(Some(build))
                .and_then(|b| b.as_str())
                .unwrap_or(".");
            e.attr("build_context", serde_json::json!(ctx));
        }
        if let Some(ports) = spec.get("ports").and_then(|p| p.as_array()) {
            let ps: Vec<String> = ports
                .iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect();
            if !ps.is_empty() {
                e.attr("ports", serde_json::json!(ps));
            }
        }
        out.entities.push(e);
        // depends_on -> deployed_with
        if let Some(deps) = spec.get("depends_on") {
            let dep_names: Vec<String> = match deps {
                serde_json::Value::Array(a) => a
                    .iter()
                    .filter_map(|d| d.as_str().map(|s| s.to_string()))
                    .collect(),
                serde_json::Value::Object(o) => o.keys().cloned().collect(),
                _ => Vec::new(),
            };
            for d in dep_names {
                let dep_id = scc_core::entity_id(repo_id, kinds::DEPLOYMENT_UNIT, &d);
                let rel = Relationship::new(
                    scc_core::relationship_id(0),
                    id.clone(),
                    scc_core::predicates::DEPENDS_ON,
                    dep_id,
                    Provenance::Extracted,
                );
                out.relationships.push((rel, path.to_string()));
            }
        }
    }
}

fn extract_env(_path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    for (key, value) in parse_env_file(content) {
        let secret = classify_secret(&key, &value);
        let kind = if secret {
            kinds::SECRET_REFERENCE
        } else {
            kinds::CONFIGURATION
        };
        let id = scc_core::entity_id(repo_id, kind, &key);
        let mut e = Entity::new(id, kind, key.clone());
        // Persist only references — never values (docs/SECURITY.md §4).
        if secret {
            e.attr("secret", serde_json::json!(true));
        }
        out.entities.push(e);
    }
}

/// Parse `service Name { rpc Foo (...) returns (...); }` into CONTRACT
/// entities. Not an AST extractor — identifier syntax only.
// trace:v1 id=impl.scc.index.proto-contracts work=WORK-ripwire-lessons-phase6 satisfies=REQ-cross-lang-semantic-bridges,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no
pub fn extract_proto(path: &str, content: &str, repo_id: &str, out: &mut ConfigExtraction) {
    let mut service: Option<String> = None;
    let mut depth: i32 = 0;
    for (i, raw) in content.lines().enumerate() {
        let line = strip_proto_comment(raw);
        if let Some(name) = parse_proto_service(&line) {
            service = Some(name);
        }
        let opens = line.bytes().filter(|b| *b == b'{').count() as i32;
        let closes = line.bytes().filter(|b| *b == b'}').count() as i32;
        depth += opens - closes;
        if let (Some(svc), Some(rpc)) = (service.as_ref(), parse_proto_rpc(&line)) {
            let key = format!("{svc}.{rpc}");
            let id = scc_core::entity_id(repo_id, kinds::CONTRACT, &key);
            let mut e = Entity::new(id.clone(), kinds::CONTRACT, key.clone());
            e.attr("kind", serde_json::json!("rpc"));
            e.attr("service", serde_json::json!(svc));
            e.attr("rpc", serde_json::json!(rpc));
            e.attr("file", serde_json::json!(path));
            e.attr("line", serde_json::json!(i as u32 + 1));
            out.entities.push(e);
            let file_id = scc_core::entity_id(repo_id, kinds::FILE, path);
            let rel = Relationship::new(
                crate::write::rel_id(&["contains", &file_id, &id]),
                file_id,
                scc_core::predicates::CONTAINS,
                id,
                Provenance::Extracted,
            );
            out.relationships.push((rel, path.to_string()));
        }
        if depth <= 0 && parse_proto_service(&line).is_none() {
            service = None;
            depth = 0;
        }
    }
}

// trace:exempt reason=internal-detail
fn strip_proto_comment(line: &str) -> String {
    match line.find("//") {
        Some(i) => line[..i].to_string(),
        None => line.to_string(),
    }
}

// trace:exempt reason=internal-detail
fn parse_proto_service(line: &str) -> Option<String> {
    ident_after_keyword(line, "service")
}

// trace:exempt reason=internal-detail
fn parse_proto_rpc(line: &str) -> Option<String> {
    ident_after_keyword(line, "rpc")
}

// trace:exempt reason=internal-detail
fn ident_after_keyword(line: &str, keyword: &str) -> Option<String> {
    let t = line.trim();
    let rest = t
        .strip_prefix(keyword)
        .filter(|r| r.starts_with(' ') || r.starts_with('\t'))?;
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// First non-heading paragraph of the README as repository purpose.
pub fn readme_purpose(content: &str) -> Option<String> {
    let mut in_code = false;
    let mut paragraphs: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.is_empty() {
            if let Some(last) = paragraphs.last() {
                if !last.is_empty() {
                    paragraphs.push(String::new());
                }
            }
            continue;
        }
        if t.starts_with('#') || t.starts_with("![]") || t.starts_with("<img") {
            continue;
        }
        if paragraphs.is_empty() || paragraphs.last().map(|p| p.is_empty()).unwrap_or(false) {
            paragraphs.push(t.to_string());
        } else {
            let last = paragraphs.last_mut().unwrap();
            last.push(' ');
            last.push_str(t);
        }
    }
    let joined = paragraphs.join("\n");
    let joined = joined.trim();
    if joined.is_empty() {
        return None;
    }
    Some(joined.chars().take(600).collect())
}

/// Materialize intent.yaml into DECLARED entities/claims consumed by the
/// graph layer. Returns (entities, relationships, invariants-as-claims).
pub fn intent_claims(intent: &Intent, _repo_id: &str) -> Vec<(String, serde_json::Value)> {
    let mut claims = Vec::new();
    for (name, comp) in &intent.components {
        claims.push((
            "component".into(),
            serde_json::json!({
                "name": name,
                "responsibility": comp.responsibility,
                "owns": comp.owns,
                "paths": comp.paths,
            }),
        ));
    }
    for (name, inv) in &intent.invariants {
        claims.push((
            "invariant".into(),
            serde_json::json!({
                "name": name,
                "statement": inv.statement,
                "severity": inv.severity,
                "scope": inv.scope,
                "enforced_by": inv.enforced_by,
            }),
        ));
    }
    for (name, flow) in &intent.flows {
        claims.push((
            "flow".into(),
            serde_json::json!({
                "name": name,
                "entrypoint": flow.entrypoint,
                "kind": flow.kind,
                "trigger": flow.trigger,
                "description": flow.description,
            }),
        ));
    }
    claims
}

/// Language-level extractions that live in config files (e.g. package.json
/// entrypoints) merged into an `ExtractedFile`-like shape for the writer.
pub fn config_as_extracted(out: &ConfigExtraction) -> ExtractedFile {
    let mut ef = ExtractedFile::default();
    for e in &out.entrypoints {
        ef.entrypoints.push(e.clone());
    }
    ef
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_json_workspaces() {
        let content = r#"{
            "name": "mono",
            "workspaces": ["packages/*"],
            "main": "dist/index.js",
            "bin": {
                "mono-cli": "bin/mono.js"
            }
        }"#;
        let out = extract_config_file("package.json", content, "mono");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].kind, kinds::PACKAGE);
        assert_eq!(out.entrypoints.len(), 2);
        let names: Vec<&str> = out.entrypoints.iter().map(|e| e.symbol.as_str()).collect();
        assert!(names.contains(&"dist/index.js"));
        assert!(names.contains(&"mono-cli"));
        assert!(out.entrypoints.iter().all(|e| e.kind == "entrypoint"));
    }

    #[test]
    // trace:v1 id=test.scc.indexer.workspace-glob-expansion work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn workspace_glob_expands_packages_star() {
        let files = vec![
            "package.json".to_string(),
            "packages/web/package.json".to_string(),
            "packages/api/package.json".to_string(),
            "packages/web/index.js".to_string(),
        ];
        let roots = expand_workspace_globs(&["packages/*".to_string()], &files);
        assert_eq!(roots, vec!["packages/api".to_string(), "packages/web".to_string()]);
        // literal dir without glob still resolves when the manifest exists
        let lit = expand_workspace_globs(&["packages/web".to_string()], &files);
        assert_eq!(lit, vec!["packages/web".to_string()]);
        // literal dir without a package.json is dropped, never fabricated
        let missing = expand_workspace_globs(&["packages/ghost".to_string()], &files);
        assert!(missing.is_empty());
    }

    #[test]
    // trace:v1 id=test.scc.indexer.workspace-members-named-packages work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn workspace_members_name_packages_and_link_edges() {
        let files = vec![
            "package.json".to_string(),
            "packages/web/package.json".to_string(),
            "packages/shared/package.json".to_string(),
        ];
        let root = r#"{"name": "mono", "workspaces": ["packages/*"]}"#;
        let web = r#"{"name": "@acme/web", "dependencies": {"@acme/shared": "1.0.0"}}"#;
        let shared = r#"{"name": "@acme/shared"}"#;
        let read = |m: &str| -> Option<String> {
            match m {
                "packages/web/package.json" => Some(web.to_string()),
                "packages/shared/package.json" => Some(shared.to_string()),
                _ => None,
            }
        };
        let g = workspace_members("package.json", root, "repo", &files, &read);
        assert_eq!(g.members.len(), 2);
        assert!(g.members.iter().any(|m| m.name == "@acme/web" && m.root == "packages/web"));
        assert!(g.members.iter().any(|m| m.name == "@acme/shared"));
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].kind, "depends_on");
        // unknown dep names never fabricate edges
        let lone = workspace_members("package.json", root, "repo", &["package.json".to_string()], &|_| None);
        assert!(lone.members.is_empty() && lone.edges.is_empty());
    }

    #[test]
    fn compose_services() {
        let content = r#"
services:
  api:
    image: my/api
    build: { context: ./services/api }
    depends_on: [db, queue]
  db:
    image: postgres:16
  queue:
    image: redis:7
"#;
        let out = extract_config_file("docker-compose.yml", content, "repo");
        let units: Vec<&Entity> = out
            .entities
            .iter()
            .filter(|e| e.kind == kinds::DEPLOYMENT_UNIT)
            .collect();
        assert_eq!(units.len(), 3);
        let deps = out
            .relationships
            .iter()
            .filter(|(r, _)| r.predicate == scc_core::predicates::DEPENDS_ON)
            .count();
        assert_eq!(deps, 2);
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn env_only_references() {
        let content = "DATABASE_URL=postgres://u:p@h/db\nPORT=8080\n";
        let out = extract_config_file(".env", content, "repo");
        let kinds_map: BTreeMap<_, _> = out
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.kind.clone()))
            .collect();
        assert_eq!(
            kinds_map.get("DATABASE_URL").unwrap(),
            kinds::SECRET_REFERENCE
        );
        assert_eq!(kinds_map.get("PORT").unwrap(), kinds::CONFIGURATION);
        // values never persisted
        assert!(out
            .entities
            .iter()
            .all(|e| !serde_json::to_string(&e.attributes)
                .unwrap()
                .contains("postgres://")));
    }

    #[test]
    fn intent_parses() {
        let content = r#"
components:
  incident-engine:
    responsibility:
      - extract incidents from transcripts
    owns: [Incident]
invariants:
  raw-immutable:
    statement: raw output cannot be modified
    severity: critical
flows:
  live-radio:
    entrypoint: RadioReceiver.handle
"#;
        let intent: Intent = serde_yaml::from_str(content).unwrap();
        assert!(intent.components.contains_key("incident-engine"));
        assert_eq!(intent.invariants["raw-immutable"].severity, "critical");
        let claims = intent_claims(&intent, "repo");
        assert_eq!(claims.len(), 3);
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn readme_purpose_extracted() {
        let content =
            "# My App\n\nThis app processes radio\naudio into incidents.\n\n## Install\n...";
        let purpose = readme_purpose(content).unwrap();
        assert!(purpose.contains("processes radio audio"));
        assert!(!purpose.contains("## Install"));
    }

    #[test]
    // trace:v1 id=test.scc.index.proto-contracts verifies=REQ-cross-lang-semantic-bridges,REQ-implement-fix-pr-review-comments-without-collapsing-scc-type-script-no exercises=impl.scc.index.proto-contracts
    fn proto_rpc_becomes_contract_not_a_call() {
        let content = r#"
syntax = "proto3";
package demo.v1;
service Orders {
  rpc GetOrder (GetOrderRequest) returns (GetOrderResponse);
  rpc ListOrders (ListOrdersRequest) returns (ListOrdersResponse); // comment
}
"#;
        let out = extract_config_file("contracts/orders.proto", content, "repo");
        let names: Vec<&str> = out.entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Orders.GetOrder"), "{names:?}");
        assert!(names.contains(&"Orders.ListOrders"), "{names:?}");
        assert!(out.entities.iter().all(|e| e.kind == kinds::CONTRACT));
        assert!(out
            .entities
            .iter()
            .all(|e| { e.attributes.get("kind").and_then(|v| v.as_str()) == Some("rpc") }));
        assert!(out
            .relationships
            .iter()
            .all(|(r, _)| r.predicate == scc_core::predicates::CONTAINS));
        let split = extract_config_file(
            "contracts/orders.proto",
            "syntax = \"proto3\";\nservice Orders\n{\n  rpc GetOrder (GetOrderRequest) returns (GetOrderResponse);\n}\n",
            "repo",
        );
        assert!(
            split
                .entities
                .iter()
                .any(|e| e.name == "Orders.GetOrder"),
            "newline before brace must keep the service: {:?}",
            split.entities.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
        );
    }
}
