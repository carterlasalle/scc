//! Engine misc operations: drift, invariants, cochange, history,
//! snapshots, lessons, import, runtime status. Value-returning; the CLI
//! renders. Moved verbatim from `scc-cli` commands.rs.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct DriftFinding {
    pub id: i64,
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub created_at: String,
}

// trace:exempt reason=internal-detail
pub fn drift(store: &scc_store::Store) -> crate::Result<Vec<DriftFinding>> {
    Ok(store
        .drift_findings(false)?
        .into_iter()
        .map(|(id, kind, severity, message, created_at)| DriftFinding { id, kind, severity, message, created_at })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct InvariantViolation {
    pub message: String,
}

// trace:exempt reason=internal-detail
pub fn check_invariants(store: &scc_store::Store) -> crate::Result<Vec<InvariantViolation>> {
    use scc_core::kinds;
    let graph = scc_graph::RealityGraph::load(store)?;
    let mut out = Vec::new();
    for r in graph.all_rels() {
        let known = |id: &str| {
            graph.entities.contains_key(id)
                || id.contains("/external_api/")
                || id.contains("/component/")
                || id.contains("/flow/")
                || id.contains("/invariant/")
        };
        if !known(&r.subject) {
            out.push(InvariantViolation { message: format!("dangling subject: {} — {}", r.subject, r.predicate) });
        }
        if !known(&r.object) {
            out.push(InvariantViolation { message: format!("dangling object: {} — {}", r.predicate, r.object) });
        }
    }
    for r in graph.all_rels() {
        if r.provenance == scc_core::Provenance::Resolved && r.evidence.is_empty() {
            out.push(InvariantViolation { message: format!("RESOLVED without evidence: {} — {}", r.subject, r.predicate) });
        }
    }
    for inv in store.invariants()? {
        if inv.severity == scc_core::Severity::Critical && inv.enforced_by.is_empty() {
            out.push(InvariantViolation { message: format!("critical invariant unenforced: {}", inv.statement) });
        }
    }
    let _ = kinds::DATA_STORE;
    Ok(out)
}

// trace:exempt reason=internal-detail
pub fn ci_check(
    store: &scc_store::Store,
    violations: &[InvariantViolation],
    max_severity: &str,
) -> crate::Result<(bool, Vec<String>)> {
    let mut lines = Vec::new();
    let mut ok = violations.is_empty();
    let allowed = match max_severity {
        "low" => 1u8,
        "medium" => 2u8,
        "high" => 3u8,
        "critical" => 4u8,
        _ => 2u8,
    };
    for (_, kind, sev, msg, _) in store.drift_findings(true)? {
        let rank = match sev.as_str() {
            "low" => 1u8,
            "medium" => 2u8,
            "high" => 3u8,
            "critical" => 4u8,
            _ => 2u8,
        };
        if rank > allowed {
            lines.push(format!("[ci:fail] [{sev}] {kind}: {msg}"));
            ok = false;
        } else {
            lines.push(format!("[ci:warn] [{sev}] {kind}: {msg}"));
        }
    }
    for v in violations {
        lines.push(v.message.clone());
    }
    if ok {
        lines.push("ci check passed".into());
    }
    Ok((ok, lines))
}

// trace:exempt reason=internal-detail
pub fn cochange(root: &std::path::Path, min_commits: u32) -> crate::Result<(Vec<scc_graph::cochange::CochangePair>, usize)> {
    let pairs = scc_graph::cochange::cochange_pairs(root, min_commits)
        .map_err(crate::EngineError::Other)?;
    let mut enriched = 0usize;
    if crate::workspace::db_path(root).exists() {
        let store = crate::workspace::open_store(root)?;
        enriched = scc_graph::cochange::enrich_components(&store, &pairs)
            .map_err(crate::EngineError::Other)?;
    }
    Ok((pairs, enriched))
}

// trace:exempt reason=internal-detail
pub fn snapshot_save(
    root: &std::path::Path,
    task: &str,
    budget: Option<usize>,
) -> crate::Result<scc_store::snapshot::ContextSnapshot> {
    let req = scc_api::TaskContextRequest { goal: task.into(), files: vec![], symbols: vec![], budget, hook: false };
    let store = crate::workspace::open_store(root)?;
    let config = crate::workspace::load_config(root)?;
    let stale = crate::workspace::stale_paths(&store)?;
    let engine = crate::workspace::open_engine(&store, &config, stale)?;
    let artifact = crate::task::build_task_context(&engine, &config, root, &req, None, None)?;
    let head = store.revisions()?.into_iter().last().map(|r| r.rev).unwrap_or(0);
    let epoch = store.model_epoch()?.composite(&head.to_string());
    let mut ids = artifact.pack.entity_ids.clone();
    ids.extend(artifact.delta_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    Ok(store.save_snapshot(scc_store::snapshot::SnapshotSave {
        task,
        epoch: &epoch,
        revision: head,
        artifact: &format!("{}{}", artifact.pack.content, artifact.delta),
        entity_ids: &ids,
        budget: artifact.token_count,
        warnings: &artifact.pack.warnings,
    })?)
}
