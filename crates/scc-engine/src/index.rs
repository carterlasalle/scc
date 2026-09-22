//! Engine index operations: full index, path refresh, resolution.
//!
//! Moved verbatim from `scc-cli` lib.rs / commands.rs: the revision-after-
//! recompile ordering contract and the no-change fast path move WITH the
//! code (parity tests enforce byte-identical reports).

use std::path::Path;

// trace:exempt reason=internal-detail
pub fn full(root: &Path, config: &scc_indexer::Config) -> crate::Result<scc_indexer::IndexReport> {
    crate::workspace::ensure_scc_ignored(root);
    crate::workspace::resilient_index(root, || {
        let (store, quarantined) = crate::workspace::open_store_recovering(root)?;
        crate::workspace::report_quarantine(&quarantined);
        let indexer = scc_indexer::Indexer::new(store, config.clone());
        let report = indexer.index()?;
        let store = crate::workspace::open_store(root)?;
        if config.index.auto_resolve {
            let _ = scc_indexer::resolver::resolve_repository(
                &store,
                root,
                scc_indexer::resolver::MAX_CALL_SITES,
            );
        }
        let unchanged = report.changed == 0 && report.removed == 0 && !config.index.auto_resolve;
        let extractor_current = store
            .revisions()
            .map(|rs| {
                rs.into_iter().last().map(|h| {
                    h.extractor_version
                        == format!(
                            "store:{};core:{}",
                            scc_store::SCHEMA_VERSION,
                            scc_core::SCHEMA_VERSION
                        )
                })
                .unwrap_or(false)
            })
            .unwrap_or(false);
        if !(unchanged && extractor_current) {
            recompile(&store)?;
        }
        let _ = store.record_current_revision_with_config(
            &scc_indexer::semantic_config_hash(config),
        )?;
        Ok(report)
    })
}

// trace:exempt reason=internal-detail
pub fn refresh_paths(root: &Path, config: &scc_indexer::Config, paths: &[String]) -> crate::Result<scc_indexer::IndexReport> {
    let (store, quarantined) = crate::workspace::open_store_recovering(root)?;
    crate::workspace::report_quarantine(&quarantined);
    let indexer = scc_indexer::Indexer::new(crate::workspace::open_store(root)?, config.clone());
    let report = indexer.refresh_paths(paths)?;
    drop(indexer);
    recompile(&store)?;
    let _ = store.record_current_revision_with_config(
        &scc_indexer::semantic_config_hash(config),
    )?;
    Ok(report)
}

// trace:exempt reason=internal-detail
pub fn recompile(store: &scc_store::Store) -> crate::Result<scc_graph::RecompileReport> {
    Ok(scc_graph::recompile(store)?)
}

// trace:exempt reason=internal-detail
pub fn resolve_and_recompile(root: &Path) -> crate::Result<scc_indexer::resolver::ResolveReport> {
    let store = crate::workspace::open_store(root)?;
    let report = scc_indexer::resolver::resolve_repository(
        &store,
        root,
        scc_indexer::resolver::MAX_CALL_SITES,
    )
    .map_err(|e| crate::EngineError::Other(e.to_string()))?;
    recompile(&store)?;
    Ok(report)
}
