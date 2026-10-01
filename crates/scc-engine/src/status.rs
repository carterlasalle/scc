//! Engine status: the `scc status` value (no println).

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct Status {
    pub repository: String,
    pub repository_id: String,
    pub remote: Option<String>,
    pub revision: String,
    pub branch: Option<String>,
    pub indexed_at: Option<String>,
    pub stats: std::collections::HashMap<String, u64>,
    pub freshness: String,
    pub stale_files: Vec<String>,
    pub stale_count: usize,
    pub analysis_quality: Option<String>,
    pub scan_stats: Option<serde_json::Value>,
    pub indexed: bool,
}

// trace:exempt reason=internal-detail
pub fn status(store: &scc_store::Store) -> crate::Result<Status> {
    let repo = store.repository();
    let stale = crate::workspace::stale_paths(store)?;
    Ok(match store.snapshot_status()? {
        Some((snap, _)) => Status {
            repository: repo.name,
            repository_id: repo.id,
            remote: repo.url,
            revision: snap.revision,
            branch: snap.branch,
            indexed_at: Some(snap.indexed_at),
            stats: store.stats()?,
            freshness: if stale.is_empty() { "CURRENT".into() } else { "STALE".into() },
            stale_files: stale.iter().take(10).cloned().collect(),
            stale_count: stale.len(),
            analysis_quality: store.meta_get("analysis_quality")?,
            scan_stats: store
                .meta_get("scan_stats")?
                .and_then(|raw| serde_json::from_str(&raw).ok()),
            indexed: true,
        },
        None => Status {
            repository: repo.name,
            repository_id: repo.id,
            remote: repo.url,
            revision: "not-indexed".into(),
            branch: None,
            indexed_at: None,
            stats: Default::default(),
            freshness: "NOT-INDEXED".into(),
            stale_files: vec![],
            stale_count: 0,
            analysis_quality: None,
            scan_stats: None,
            indexed: false,
        },
    })
}

/// Scan explanation: which files the indexer would index and why
/// (languages, ignore rules, budgets). The `scc scan` value — transports
/// render it; the engine owns the derivation.
// trace:v1 id=impl.scc-engine-status.scan work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn scan(
    root: &std::path::Path,
    config: &scc_indexer::Config,
    path: Option<&str>,
) -> crate::Result<serde_json::Value> {
    let exp = scc_indexer::scan::explain_scan(root, &config.index)
        .map_err(|e| crate::EngineError::Other(e.to_string()))?;
    let indexed: Vec<serde_json::Value> = exp
        .indexed
        .into_iter()
        .filter(|f| path.filter(|p| !p.is_empty()).map(|p| f.path == p || f.path.starts_with(p)).unwrap_or(true))
        .map(|f| {
            serde_json::json!({"path": f.path, "language": f.language.as_str(), "kind": f.kind.as_str(), "bytes": f.size})
        })
        .collect();
    let skipped: Vec<serde_json::Value> = exp
        .skipped
        .into_iter()
        .map(|sk| serde_json::json!({"path": sk.path, "reason": sk.reason, "rule": sk.rule}))
        .collect();
    let st = &exp.stats;
    Ok(serde_json::json!({"indexed": indexed, "skipped": skipped,
        "stats": {"discovered": st.discovered, "indexed": st.indexed, "ignored": st.ignored,
                  "unsupported": st.unsupported, "oversized": st.oversized, "unreadable": st.unreadable,
                  "symlink_escape": st.symlink_escape}}))
}
