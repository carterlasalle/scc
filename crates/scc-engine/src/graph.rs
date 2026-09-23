//! Engine graph operations: search, listings, recompile.
//!
//! Returns VALUES (entities, pairs, packs); transports render them.
//! The FTS+LIKE fallback contract moves with the code.

use scc_api::QueryRequest;

// trace:exempt reason=internal-detail
pub struct QueryHit {
    pub entities: Vec<scc_core::Entity>,
    /// (name, signature, kind, file, start_line)
    pub symbols: Vec<(String, String, String, String, u32)>,
}

// trace:exempt reason=internal-detail
pub fn query(store: &scc_store::Store, req: &QueryRequest) -> crate::Result<QueryHit> {
    let limit = req.limit.max(1);
    let entities = store.search_entities(&req.query, limit)?;
    let symbols = store.search_symbols(&req.query, limit)?;
    let (entities, symbols) = if entities.is_empty() && symbols.is_empty() {
        (
            store.search_entities_like(&req.query, limit)?,
            store.search_symbols_like(&req.query, limit)?,
        )
    } else {
        (entities, symbols)
    };
    Ok(QueryHit { entities, symbols })
}

// trace:exempt reason=internal-detail
pub fn components(store: &scc_store::Store) -> crate::Result<Vec<scc_core::Entity>> {
    Ok(store.components()?)
}

// trace:exempt reason=internal-detail
pub fn flows(store: &scc_store::Store) -> crate::Result<Vec<scc_core::Flow>> {
    Ok(store.flows()?)
}

// trace:exempt reason=internal-detail
pub fn relationships(
    store: &scc_store::Store,
    subject: Option<&str>,
    predicate: Option<&str>,
    limit: usize,
) -> crate::Result<Vec<scc_core::Relationship>> {
    let mut out = if let Some(s) = subject {
        store.relationships_for(s)?
    } else {
        store.all_relationships()?
    };
    if let Some(p) = predicate {
        out.retain(|r| r.predicate == p);
    }
    out.truncate(limit.max(1));
    Ok(out)
}
