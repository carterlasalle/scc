//! Engine snapshots: get + diff (typed values).

// trace:exempt reason=internal-detail
pub fn get(store: &scc_store::Store, id: &str) -> crate::Result<Option<scc_store::snapshot::ContextSnapshot>> {
    Ok(store.load_snapshot(id)?)
}

// trace:exempt reason=internal-detail
pub fn diff(store: &scc_store::Store, id: &str) -> crate::Result<Option<scc_store::snapshot::SnapshotDiff>> {
    Ok(store.diff_snapshot(id)?)
}
