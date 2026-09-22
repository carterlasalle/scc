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
