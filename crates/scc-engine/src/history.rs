//! Engine history: revisions + semantic diff (typed values).

// trace:exempt reason=internal-detail
pub fn revisions(store: &scc_store::Store) -> crate::Result<Vec<scc_store::history::GraphRevision>> {
    Ok(store.revisions()?)
}

// trace:exempt reason=internal-detail
pub fn diff(store: &scc_store::Store, from: i64, to: i64) -> crate::Result<scc_store::history::SemanticDelta> {
    Ok(store.semantic_diff(from, to)?)
}
