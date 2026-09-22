//! Engine state: lessons, beads, evidence import, runtime.
//!
//! Value-returning operations; the CLI renders. Import bumps the evidence
//! epoch and recompiles the derived layer (moved with the code).

use std::io::Write;
use std::path::{Path, PathBuf};

// trace:exempt reason=internal-detail
pub fn lessons_add(root: &Path, text: &str) -> crate::Result<(String, PathBuf)> {
    let dir = crate::workspace::scc_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("lessons.jsonl");
    let n = std::fs::read_to_string(&path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let id = format!("lesson-{}", n + 1);
    let record = serde_json::json!({
        "id": id,
        "text": text,
        "created_at": scc_core::now_rfc3339(),
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| crate::EngineError::Other(format!("lessons: {e}")))?;
    writeln!(f, "{record}").map_err(|e| crate::EngineError::Other(format!("lessons: {e}")))?;
    Ok((id, path))
}

// trace:exempt reason=internal-detail
pub fn lessons_list(root: &Path, limit: usize) -> crate::Result<Vec<(String, Vec<String>)>> {
    let store = crate::workspace::open_store(root)?;
    Ok(scc_indexer::adapters::hindsight::lessons(&store, limit))
}

// trace:exempt reason=internal-detail
pub fn beads(root: &Path, limit: usize) -> crate::Result<Vec<String>> {
    Ok(scc_indexer::adapters::beads::active_beads(root, limit))
}

// trace:exempt reason=internal-detail
pub fn import_evidence(root: &Path, format: &str, file: &str) -> crate::Result<scc_indexer::adapters::ImportReport> {
    let store = crate::workspace::open_store(root)?;
    let report = match format {
        "scip" => scc_indexer::adapters::import_scip(&store, std::path::Path::new(file)),
        "ccg" => scc_indexer::adapters::import_ccg(&store, std::path::Path::new(file)),
        "gitnexus" => scc_indexer::adapters::gitnexus::import_gitnexus(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.symbols,
                calls: r.edges,
                imports: 0,
                errors: r.errors,
            }),
        "beads" => scc_indexer::adapters::beads::import_beads(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.tasks,
                calls: r.dependencies,
                imports: r.active,
                errors: r.errors,
            }),
        "cbm" => scc_indexer::adapters::cbm::import_cbm(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.symbols,
                calls: r.relationships,
                imports: 0,
                errors: r.errors,
            }),
        "hindsight" => scc_indexer::adapters::hindsight::import_hindsight(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.lessons,
                calls: 0,
                imports: 0,
                errors: r.errors,
            }),
        "tracelayer" => scc_indexer::adapters::tracelayer::import_tracelayer(&store, std::path::Path::new(file))
            .map(|r| scc_indexer::adapters::ImportReport {
                symbols: r.requirements + r.implementations + r.tests + r.decisions,
                calls: r.relationships,
                imports: r.work_items,
                errors: r.errors,
            }),
        other => {
            return Err(crate::EngineError::Other(format!(
                "unknown import format '{other}' (use scip, ccg, gitnexus, beads, cbm, hindsight, or tracelayer)"
            )))
        }
    }
    .map_err(crate::EngineError::Other)?;
    store.bump_epoch(scc_store::ModelEpochKind::Evidence)?;
    crate::index::recompile(&store)?;
    Ok(report)
}

// trace:exempt reason=internal-detail
pub fn runtime_edges(root: &Path) -> crate::Result<Vec<scc_indexer::runtime::RuntimeEdge>> {
    let store = crate::workspace::open_store(root)?;
    scc_indexer::runtime::runtime_edges(&store).map_err(crate::EngineError::Other)
}

// trace:exempt reason=internal-detail
pub fn ingest_runtime(root: &Path, body: &str) -> crate::Result<()> {
    let store = crate::workspace::open_store(root)?;
    if body.contains("resourceSpans") {
        scc_indexer::runtime::ingest_otlp_json(&store, body)
            .map_err(crate::EngineError::Other)?;
        return Ok(());
    }
    scc_indexer::runtime::ingest_simple_edges(&store, body)
        .map_err(crate::EngineError::Other)?;
    Ok(())
}

// trace:exempt reason=internal-detail
pub fn reconcile(root: &Path) -> crate::Result<scc_indexer::runtime::Reconciliation> {
    let store = crate::workspace::open_store(root)?;
    scc_indexer::runtime::reconcile(&store).map_err(crate::EngineError::Other)
}
