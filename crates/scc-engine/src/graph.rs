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

/// Multi-step traversal (§16): resolve the start set (kind + LIKE name,
/// union explicit ids), then walk each step's directed edges. Returns the
/// final frontier entities plus every traversed relationship. Frontier is
/// a set (visited ids never re-expand); per-entity, per-step edge fan-out
/// is capped by `step.limit` (default 50); output capped by `limit`.
/// Unknown directions fail loudly — never silently walk the wrong way.
// trace:v1 id=impl.scc-engine-graph.traverse work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn traverse(
    compiler: &scc_context::ContextCompiler<'_>,
    req: &scc_api::TraverseRequest,
) -> crate::Result<(Vec<scc_core::Entity>, Vec<scc_core::Relationship>)> {
    let store = compiler.store;
    use std::collections::{BTreeMap, BTreeSet};
    let mut start_ids: BTreeSet<String> = BTreeSet::new();
    if let Some(kind) = req.kind.as_deref().filter(|k| !k.is_empty()) {
        let needle = req.name.clone().unwrap_or_default();
        for e in store.search_entities_like_kind(kind, &needle, 200)? {
            start_ids.insert(e.id);
        }
    } else if let Some(name) = req.name.as_deref().filter(|n| !n.is_empty()) {
        for e in store.search_entities_like(name, 200)? {
            start_ids.insert(e.id);
        }
        for (n, _, _, _, _) in store.search_symbols_like(name, 200)? {
            for e in store.search_entities_like(&n, 20)? {
                start_ids.insert(e.id);
            }
        }
    }
    for id in &req.from_ids {
        start_ids.insert(id.clone());
    }
    for step in &req.steps {
        if !["out", "in", "both"].contains(&step.dir.as_str()) {
            return Err(crate::EngineError::Other(format!(
                "unknown traverse direction '{}' (use out|in|both)",
                step.dir
            )));
        }
    }
    if start_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let entities: BTreeMap<String, scc_core::Entity> = store
        .all_entities()?
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let mut frontier: BTreeSet<String> = start_ids;
    let mut visited: BTreeSet<String> = frontier.clone();
    let mut rels: Vec<scc_core::Relationship> = Vec::new();
    for step in &req.steps {
        let cap = if step.limit == 0 { 50 } else { step.limit };
        let mut next: BTreeSet<String> = BTreeSet::new();
        let mut step_rels: Vec<scc_core::Relationship> = Vec::new();
        // Deterministic: store queries order by id; frontier is a BTreeSet.
        for id in &frontier {
            // Trusted (default): the compiler's TrustedGraphView filters
            // STALE / low-confidence INFERRED / disallowed provenance —
            // the same authority every context consumer traverses (§§1.2,
            // 125). Raw (trusted_only=false) walks store edges verbatim,
            // explicitly — never silently.
            let mut edges: Vec<scc_core::Relationship> = if req.trusted_only {
                match step.dir.as_str() {
                    "out" => compiler.view.out_edges(id),
                    "in" => compiler.view.in_edges(id),
                    "both" => {
                        let mut e = compiler.view.out_edges(id);
                        e.extend(compiler.view.in_edges(id));
                        e.sort_by(|a, b| a.id.cmp(&b.id));
                        e
                    }
                    _ => unreachable!("direction validated above"),
                }
                .into_iter()
                .cloned()
                .collect()
            } else {
                match step.dir.as_str() {
                    "out" => store.relationships_for(id)?,
                    "in" => store.relationships_to(id)?,
                    "both" => {
                        let mut e = store.relationships_for(id)?;
                        e.extend(store.relationships_to(id)?);
                        e.sort_by(|a, b| a.id.cmp(&b.id));
                        e
                    }
                    _ => unreachable!("direction validated above"),
                }
            };
            if let Some(p) = step.predicate.as_deref().filter(|s| !s.is_empty()) {
                edges.retain(|r| r.predicate == p);
            }
            edges.truncate(cap);
            for r in edges {
                let land = (if r.subject == *id { &r.object } else { &r.subject }).clone();
                if let Some(k) = step.where_kind.as_deref().filter(|s| !s.is_empty()) {
                    match entities.get(land.as_str()) {
                        Some(e) if e.kind == k => {}
                        _ => continue,
                    }
                }
                step_rels.push(r);
                if visited.insert(land.clone()) {
                    next.insert(land);
                }
            }
        }
        rels.extend(step_rels);
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    let mut out_entities: Vec<scc_core::Entity> = frontier
        .iter()
        .filter_map(|id| entities.get(id).cloned())
        .collect();
    let limit = if req.limit == 0 { 100 } else { req.limit };
    out_entities.truncate(limit);
    Ok((out_entities, rels))
}
