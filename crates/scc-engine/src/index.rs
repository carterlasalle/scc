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
            recompile_with_signals(root, config, &store)?;
        }
        let _ = store.record_current_revision_with_config(
            &scc_indexer::semantic_config_hash(config),
        )?;
        let _ = store.prune_revisions(config.history.max_revisions);
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
    // C1a true no-op fast path: hashes identical AND no deletions means
    // the store (source facts) and therefore every derived fact is
    // unchanged — recompiling would rebuild identical tables and record
    // an identical revision. Skip both; the report already carries
    // mutated=false so callers can observe the fast path.
    if !report.mutated {
        return Ok(report);
    }
    // C1b: derive the affected closure from the true changed set. The
    // pipeline still runs whole-repo until per-stage merge writes land;
    // the closure bound below is the observable contract the merge work
    // must exploit (tight closure + full pipeline = the remaining gap).
    let signals = component_signals(root, config);
    let fresh = crate::workspace::open_store(root)?;
    let (_scoped, closure) = scc_graph::recompile_scoped_with_owners(
        &fresh,
        &report.affected_files,
        &report.affected_components,
        signals,
    )?;
    eprintln!(
        "[scc] refresh closure: {} file(s) -> {} component(s), {} flow(s){}",
        report.affected_files.len(),
        closure.components.len(),
        closure.flows.len(),
        if closure.complete { " (UNBOUNDED: full pipeline required)" } else { "" },
    );
    recompile_with_signals(root, config, &store)?;
    let _ = store.record_current_revision_with_config(
        &scc_indexer::semantic_config_hash(config),
    )?;
    let _ = store.prune_revisions(config.history.max_revisions);
    Ok(report)
}

// trace:exempt reason=internal-detail
pub fn recompile(store: &scc_store::Store) -> crate::Result<scc_graph::RecompileReport> {
    Ok(scc_graph::recompile(store)?)
}

/// Recompile with plugin component signals (§31): collects
/// `components.signals` from active plugins, then runs the pipeline with
/// them. Signal failures degrade to empty (the collector never errors).
// trace:exempt reason=internal-detail
pub fn recompile_with_signals(
    root: &std::path::Path,
    config: &scc_indexer::Config,
    store: &scc_store::Store,
) -> crate::Result<scc_graph::RecompileReport> {
    let signals = component_signals(root, config);
    Ok(scc_graph::CompilationPipeline::new(store)
        .component_signals(signals)
        .run()?)
}

/// Collect component signals (§31 ComponentSignalProvider) from plugins
/// answering the `components.signals` operation with
/// `{"signals": [{name, dirs, provider?}]}`. Unknown/dead plugins, bad
/// shapes, and empty names/dirs degrade to abstention (never fail the
/// recompile — a contributor that cannot name a component abstains).
// trace:exempt reason=internal-detail
pub fn component_signals(
    root: &std::path::Path,
    config: &scc_indexer::Config,
) -> Vec<scc_graph::components::ComponentSignal> {
    let mut ap = crate::plugins::active(root, config);
    let mut out = Vec::new();
    for plug in ap.plugins.clone() {
        let pid = plug.manifest.id.clone();
        let provides = plug.manifest.operations.iter().any(|o| o == "components.signals")
            || plug.manifest.extensions.iter().any(|e| e.extension_type == "component-signal");
        if !provides {
            continue;
        }
        match scc_plugin_host::call(&plug, "components.signals", serde_json::json!({}), None) {
            Ok(v) => {
                if let Some(arr) = v.get("signals").and_then(|x| x.as_array()) {
                    for e in arr {
                        let name = e.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let dirs: Vec<String> = e.get("dirs").and_then(|x| x.as_array()).map(|a| {
                            a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()
                        }).unwrap_or_default();
                        if name.is_empty() || dirs.is_empty() {
                            continue;
                        }
                        out.push(scc_graph::components::ComponentSignal {
                            name,
                            dirs,
                            provider: e.get("provider").and_then(|x| x.as_str()).unwrap_or(&pid).to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                ap.diagnostics.push(scc_plugin_host::PluginDiagnostic {
                    plugin: pid, operation: "components.signals".into(),
                    error: e.to_string(), action: "skipped".into(),
                });
            }
        }
    }
    out
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
    let config = crate::workspace::load_config(root)?;
    recompile_with_signals(root, &config, &store)?;
    Ok(report)
}
