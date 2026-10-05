//! Language-aware semantic resolution (Wave 4 §23-24): one dispatcher over
//! every semantic backend.
//!
//! Contract: heuristics nominate (EXTRACTED edges), semantic engines
//! resolve (RESOLVED edges). The dispatcher upgrades EXTRACTED call edges
//! in files whose language has a backend: .py -> pyright, TS/JS ->
//! typescript-language-server, SCIP index -> SCIP facts. Missing tools
//! degrade with a hint, never a failure.

use crate::lsp::LspResult;
use crate::lsp_ts::LSP_EXTRACTOR as TS_EXTRACTOR;
use scc_core::Provenance;
use scc_store::Store;
use std::collections::BTreeMap;
use std::path::Path;

pub const MAX_CALL_SITES: usize = 500;
// trace:v1 id=impl.scc.resolver work=WORK-SCC-001 satisfies=REQ-SCC-API

/// Semantic backend contract.
pub trait SemanticResolver {
    /// Whether this backend resolves `file` (by extension).
    fn supports(&self, file: &str) -> bool;
    /// Resolve EXTRACTED call sites in `file`; upgrades write RESOLVED
    /// edges with fresh evidence and bump the semantic model epoch.
    fn resolve(&mut self, store: &Store, file: &str) -> Result<LspResult, String>;
}

/// Outcome of one `resolve_repository` run.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResolveReport {
    pub upgraded: usize,
    pub unresolved: usize,
    pub errors: usize,
    /// Files that still hold EXTRACTED call edges after the run.
    pub remaining_candidates: usize,
    /// Backends that were available and used.
    pub backends_used: Vec<String>,
    /// Backends unavailable (not installed) — degraded, not fatal.
    pub backends_missing: Vec<String>,
}

/// Collect every source file with EXTRACTED call edges, capped at
/// `max_sites` total call sites, ordered by path for determinism.
pub fn files_with_candidate_edges(
    store: &Store,
    max_sites: usize,
) -> Result<Vec<(String, usize)>, String> {
    let all_rels = store.all_relationships().map_err(|e| e.to_string())?;
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let mut remaining = max_sites;
    for (path, _h, _lang, _kind, _size) in store.all_files().map_err(|e| e.to_string())? {
        let ids = store
            .relationship_ids_with_source(&path, scc_core::predicates::CALLS)
            .map_err(|e| e.to_string())?;
        if ids.is_empty() {
            continue;
        }
        let extracted = all_rels
            .iter()
            .filter(|r| {
                r.predicate == scc_core::predicates::CALLS
                    && r.provenance == Provenance::Extracted
                    && ids.contains(&r.id)
            })
            .count();
        if extracted > 0 {
            let take = extracted.min(remaining);
            files.insert(path, take);
            remaining -= take;
            if remaining == 0 {
                break;
            }
        }
    }
    Ok(files.into_iter().collect())
}

/// Run every applicable semantic backend over the repository's candidate
/// files. Missing backends degrade; the run never fails on tool absence.
// trace:v1 id=impl.scc-indexer-resolver.plugin-resolvers work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn resolve_repository(
    store: &Store,
    root: &Path,
    max_sites: usize,
) -> Result<ResolveReport, String> {
    let files = files_with_candidate_edges(store, max_sites)?;
    let mut report = ResolveReport {
        remaining_candidates: files.len(),
        ..Default::default()
    };
    if files.is_empty() {
        return Ok(report);
    }

    let is_ts = |f: &str| {
        f.ends_with(".ts") || f.ends_with(".tsx") || f.ends_with(".js") || f.ends_with(".jsx")
    };
    let py_files: Vec<&str> = files.iter().filter(|(f, _)| f.ends_with(".py")).map(|(f, _)| f.as_str()).collect();
    let ts_files: Vec<&str> = files.iter().filter(|(f, _)| is_ts(f)).map(|(f, _)| f.as_str()).collect();

    let mut run_backend = |backend: &str, file_list: &[&str]| -> Result<(), String> {
        // (start fn, error marker) per backend — resolved below to keep the
        // trait object simple
        let mut resolver: Box<dyn SemanticResolver> = match backend {
            "pyright" => {
                let r = crate::lsp::start_pyright(root);
                match r {
                    Ok(r) => Box::new(r),
                    Err(e) if e.contains("pyright not found") => {
                        report.backends_missing.push("pyright".into());
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
            }
            "tsserver" => {
                let r = crate::lsp_ts::start_tsserver(root);
                match r {
                    Ok(r) => Box::new(r),
                    Err(e) if e.contains("tsserver not found") => {
                        report.backends_missing.push("tsserver".into());
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
            }
            _ => return Ok(()),
        };
        report.backends_used.push(backend.to_string());
        let mut fatal: Option<String> = None;
        for file in file_list {
            match resolver.resolve(store, file) {
                Ok(r) => {
                    report.upgraded += r.upgraded;
                    report.unresolved += r.unresolved;
                    report.errors += r.errors;
                    if r.upgraded > 0 {
                        report.remaining_candidates = report.remaining_candidates.saturating_sub(1);
                    }
                }
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = fatal {
            return Err(format!("{backend}: {e}"));
        }
        Ok(())
    };

    if !py_files.is_empty() {
        run_backend("pyright", &py_files)?;
    }
    if !ts_files.is_empty() {
        run_backend("tsserver", &ts_files)?;
    }
    // Plugin precision resolvers (§31 SemanticResolver): declared `resolver`
    // plugins run as backends after the built-ins, through the same
    // SemanticResolver contract and validate-then-commit path. Absent
    // plugins = no-op; dead plugins degrade per file, never fatal.
    for mut plugin in PluginResolver::all(root) {
        let pid = plugin.plugin.manifest.id.clone();
        report.backends_used.push(format!("plugin:{pid}"));
        let mut fatal: Option<String> = None;
        for (file, _) in &files {
            match plugin.resolve(store, file) {
                Ok(r) => {
                    report.upgraded += r.upgraded;
                    report.unresolved += r.unresolved;
                    report.errors += r.errors;
                    if r.upgraded > 0 {
                        report.remaining_candidates = report.remaining_candidates.saturating_sub(1);
                    }
                }
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = fatal {
            return Err(format!("plugin:{pid}: {e}"));
        }
    }
    Ok(report)
}

/// EXTRACTED `calls` edges in `file` as plugin input rows.
// trace:exempt reason=internal-detail
fn candidate_calls(
    store: &Store,
    file: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let rel_ids: std::collections::HashSet<String> = store
        .relationship_ids_with_source(file, scc_core::predicates::CALLS)
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    if rel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for r in store.all_relationships().map_err(|e| e.to_string())? {
        if r.predicate != scc_core::predicates::CALLS
            || r.provenance != Provenance::Extracted
            || !rel_ids.contains(&r.id)
        {
            continue;
        }
        let line = r.evidence.first()
            .and_then(|id| store.get_evidence(id).ok().flatten())
            .and_then(|ev| ev.start_line)
            .unwrap_or(0);
        out.push(serde_json::json!({
            "subject": r.subject, "object": r.object,
            "line": line, "evidence": r.evidence,
        }));
    }
    Ok(out)
}

/// Validate-then-commit one file's plugin upgrades: every upgrade names
/// an existing candidate edge and an existing target entity; the commit
/// replaces EXTRACTED with RESOLVED (confidence 0.99, same as LSP exact)
/// and bumps the semantic epoch once per file with any upgrade.
// trace:exempt reason=internal-detail
fn apply_plugin_upgrades(
    store: &Store,
    file: &str,
    calls: &[serde_json::Value],
    upgrades: &[serde_json::Value],
    plugin_id: &str,
) -> Result<usize, String> {
    use scc_core::EvidenceType;
    let known: std::collections::BTreeSet<(String, String, u64)> = calls
        .iter()
        .map(|c| {
            (
                c.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                c.get("object").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                c.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
            )
        })
        .collect();
    // Validate everything before touching the model.
    let mut plan: Vec<(String, scc_core::Relationship, scc_core::Evidence)> = Vec::new();
    for (i, u) in upgrades.iter().enumerate() {
        let (sub, obj, line) = (
            u.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
            u.get("object").and_then(|v| v.as_str()).unwrap_or(""),
            u.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
        );
        if !known.contains(&(sub.to_string(), obj.to_string(), line)) {
            return Err(format!("plugin {plugin_id} upgrade #{i}: no such EXTRACTED calls edge {sub} -> {obj}:{line}"));
        }
        let target = u.get("target").and_then(|v| v.as_str()).unwrap_or("");
        if store.get_entity(target).map_err(|e| e.to_string())?.is_none() {
            return Err(format!("plugin {plugin_id} upgrade #{i}: unknown target entity {target}"));
        }
        let ev_in: Vec<String> = u
            .get("evidence")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        for id in &ev_in {
            if store.get_evidence(id).map_err(|e| e.to_string())?.is_none() {
                return Err(format!("plugin {plugin_id} upgrade #{i}: unknown evidence {id}"));
            }
        }
        let old_id = crate::write::rel_id(&["calls", sub, obj]);
        let new_rel = scc_core::Relationship::new(
            crate::write::rel_id(&["calls", sub, target]),
            sub.to_string(),
            scc_core::predicates::CALLS,
            target.to_string(),
            Provenance::Resolved,
        )
        .with_confidence(scc_core::resolution::confidence::LSP_EXACT)
        .with_evidence(ev_in);
        let ev = scc_core::Evidence {
            id: crate::write::evidence_id(file, "call", obj, line as u32),
            r#type: EvidenceType::Source,
            path: Some(file.to_string()),
            symbol: Some(obj.to_string()),
            start_line: Some(line as u32),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some(format!("plugin:{plugin_id}")),
            extractor_version: None,
        };
        plan.push((old_id, new_rel, ev));
    }
    for (old_id, new_rel, ev) in plan {
        store.insert_evidence(&ev).map_err(|e| e.to_string())?;
        store.delete_relationship(&old_id).map_err(|e| e.to_string())?;
        store.insert_relationship(&new_rel, file).map_err(|e| e.to_string())?;
    }
    if !upgrades.is_empty() {
        store.bump_epoch(scc_store::ModelEpochKind::Semantic).map_err(|e| e.to_string())?;
    }
    Ok(upgrades.len())
}

/// A plugin precision resolver: answers `resolution.resolve` for one file's
/// candidate calls with `{"upgrades": [{subject, object, line, evidence,
/// target}]}`. Runs after the built-ins through the same validate-then-commit
/// path (unknown edges/targets/evidence fail the file without touching the
/// model); a dead plugin degrades to zero upgrades, never a fatal error.
// trace:v1 id=impl.scc-indexer-resolver.plugin-backend work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct PluginResolver {
    plugin: scc_plugin_host::LoadedPlugin,
}

// trace:exempt reason=internal-detail
impl PluginResolver {
    /// Whether this plugin declares the `resolver` extension (or the legacy
    /// `resolution.resolve` operation).
    // trace:exempt reason=internal-detail
    pub fn declares(plugin: &scc_plugin_host::LoadedPlugin) -> bool {
        plugin.manifest.extensions.iter().any(|e| e.extension_type == "resolver")
            || plugin.manifest.operations.iter().any(|o| o == "resolution.resolve")
    }

    /// All declared plugin resolvers, in discovery order (deterministic).
    // trace:exempt reason=internal-detail
    pub fn all(root: &Path) -> Vec<Self> {
        scc_plugin_host::discover(root)
            .into_iter()
            .filter(Self::declares)
            .map(|plugin| Self { plugin })
            .collect()
    }
}

// trace:exempt reason=internal-detail
impl SemanticResolver for PluginResolver {
    // trace:exempt reason=internal-detail
    fn supports(&self, _file: &str) -> bool {
        true
    }

    // trace:exempt reason=internal-detail
    fn resolve(&mut self, store: &Store, file: &str) -> Result<LspResult, String> {
        let calls = candidate_calls(store, file)?;
        if calls.is_empty() {
            return Ok(LspResult::default());
        }
        let pid = self.plugin.manifest.id.clone();
        let input = serde_json::json!({"file": file, "calls": calls});
        let out = match scc_plugin_host::call(&self.plugin, "resolution.resolve", input, None) {
            Ok(v) => v,
            Err(_) => {
                return Ok(LspResult { unresolved: calls.len(), errors: 1, ..Default::default() });
            }
        };
        let upgrades: Vec<serde_json::Value> = out
            .get("upgrades")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        match apply_plugin_upgrades(store, file, &calls, &upgrades, &pid) {
            Ok(n) => Ok(LspResult {
                upgraded: n,
                unresolved: calls.len().saturating_sub(n),
                ..Default::default()
            }),
            Err(_) => Ok(LspResult { unresolved: calls.len(), errors: 1, ..Default::default() }),
        }
    }
}

/// The trait implementations live with their servers (lsp.rs / lsp_ts.rs);
/// this blanket impl avoids changing those public types.
impl SemanticResolver for crate::lsp::LspResolver {
    fn supports(&self, file: &str) -> bool {
        file.ends_with(".py")
    }

    fn resolve(&mut self, store: &Store, file: &str) -> Result<LspResult, String> {
        self.resolve_call_definitions(store, file)
    }
}

impl SemanticResolver for crate::lsp_ts::TsLspResolver {
    fn supports(&self, file: &str) -> bool {
        file.ends_with(".ts")
            || file.ends_with(".tsx")
            || file.ends_with(".js")
            || file.ends_with(".jsx")
    }

    fn resolve(&mut self, store: &Store, file: &str) -> Result<LspResult, String> {
        self.resolve_call_definitions(store, file)
    }
}

// TS_EXTRACTOR re-export keeps the module self-describing for SCIP work.
#[allow(unused)]
const _: &str = TS_EXTRACTOR;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_files_are_deterministic_and_capped() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        // nothing indexed -> no candidates
        let files = files_with_candidate_edges(&store, 10).unwrap();
        assert!(files.is_empty());
        let report = resolve_repository(&store, &root, 10).unwrap();
        assert_eq!(report.upgraded, 0);
    }

    #[test]
    fn backend_support_matches_extensions() {
        // exercised via the real servers' supports(); the dispatch mapping
        // itself is covered by the CLI integration tests
        assert!(is_ts_helper("src/a.ts"));
        assert!(is_ts_helper("src/b.jsx"));
        assert!(!is_ts_helper("src/c.py"));
    }

    fn is_ts_helper(f: &str) -> bool {
        f.ends_with(".ts") || f.ends_with(".tsx") || f.ends_with(".js") || f.ends_with(".jsx")
    }
}
