//! Engine export operations: System IR, JSONL, CCG, capsule.
//!
//! Moved verbatim from `scc-cli` lib.rs / compress.rs. The CLI wrappers
//! delegate; derivation is byte-identical.

use scc_core::kinds;

// trace:exempt reason=internal-detail
pub fn system_ir(store: &scc_store::Store) -> crate::Result<scc_core::SystemIr> {
    let repository = store.repository();
    let snapshot = store
        .latest_snapshot()?
        .unwrap_or(scc_core::Snapshot {
            revision: "not-indexed".into(),
            branch: None,
            indexed_at: scc_core::now_rfc3339(),
        });
    let mut ir = scc_core::SystemIr::empty(repository, snapshot);
    let mut seen = std::collections::HashSet::new();
    for e in store.all_entities()? {
        if seen.insert(e.id.clone()) {
            ir.entities.push(e);
        }
    }
    ir.relationships = store.all_relationships()?;
    ir.flows = store.flows()?;
    ir.invariants = store.invariants()?;
    ir.evidence = store.all_evidence()?;
    Ok(ir)
}

// trace:exempt reason=internal-detail
pub fn jsonl(ir: &scc_core::SystemIr) -> crate::Result<Vec<String>> {
    let mut out = Vec::new();
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "repository", "repository": ir.repository
    }))?);
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "snapshot", "snapshot": ir.snapshot, "schema_version": ir.schema_version
    }))?);
    for e in &ir.entities {
        out.push(serde_json::to_string(&serde_json::json!({"type": "entity", "entity": e}))?);
    }
    for r in &ir.relationships {
        out.push(serde_json::to_string(&serde_json::json!({"type": "relationship", "relationship": r}))?);
    }
    for f in &ir.flows {
        out.push(serde_json::to_string(&serde_json::json!({"type": "flow", "flow": f}))?);
    }
    for i in &ir.invariants {
        out.push(serde_json::to_string(&serde_json::json!({"type": "invariant", "invariant": i}))?);
    }
    for e in &ir.evidence {
        out.push(serde_json::to_string(&serde_json::json!({"type": "evidence", "evidence": e}))?);
    }
    Ok(out)
}

// trace:exempt reason=internal-detail
pub fn ccg(ir: &scc_core::SystemIr) -> crate::Result<serde_json::Value> {
    let l1: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| {
            e.kind == kinds::COMPONENT
                || e.kind == kinds::SERVICE
                || e.kind == kinds::DATA_STORE
                || e.kind == kinds::DEPLOYMENT_UNIT
                || e.kind == kinds::EXTERNAL_API
        })
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.kind,
                "attributes": e.attributes,
            })
        })
        .collect();
    let l2: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| e.kind == kinds::SYMBOL)
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.attributes.get("kind").cloned().unwrap_or(serde_json::json!("symbol")),
                "file": e.attributes.get("file").cloned().unwrap_or_default(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema": "ccg",
        "producer": "scc",
        "repository": ir.repository,
        "snapshot": ir.snapshot,
        "layers": {
            "L0": {
                "manifest": {
                    "repository": ir.repository.name,
                    "revision": ir.snapshot.revision,
                    "entity_count": ir.entities.len(),
                    "relationship_count": ir.relationships.len(),
                }
            },
            "L1": { "architecture": l1 },
            "L2": { "symbols": l2 },
        }
    }))
}

// trace:exempt reason=internal-detail
pub fn capsule(root: &std::path::Path) -> crate::Result<String> {
    let store = crate::workspace::open_store(root)?;
    let config = crate::workspace::load_config(root)?;
    let stale = crate::workspace::stale_paths(&store)?;
    let engine = crate::workspace::open_engine(&store, &config, stale)?;
    let overview = engine.context().overview()?;
    let revision = overview.repository_revision.clone();
    let repo = store.repository();
    let paths: Vec<String> = store
        .all_files()
        .unwrap_or_default()
        .into_iter()
        .map(|(p, _, _, _, _)| p)
        .collect();
    let skeleton = scc_context::skeleton::build_skeleton(
        &paths,
        scc_context::skeleton::skeleton_budget(config.context.startup_tokens),
    );
    Ok(format!(
        "<!-- SCC-CAPSULE v1 repo={} revision={} generated={} -->\n# SYSTEM CAPSULE\n\n{}\n## REPOSITORY SKELETON\n\n{}\n\n(Physical layout above; run `scc context startup` for the fused live architecture.)\n",
        repo.id,
        revision,
        scc_core::now_rfc3339(),
        overview.content,
        skeleton.text
    ))
}

/// Complete live model (§35): repository, snapshot, epoch, files, symbols,
/// entities, relationships, evidence, components, flows, flow graphs,
/// invariants, and stats in one structured envelope.
///
/// This is the programmatic `give me everything SCC knows`; the export
/// formats stay the interoperable representations. Sections reuse the same
/// store getters as `system_ir` — one derivation, one envelope.
// trace:v1 id=impl.scc-engine-exports.model-get work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn model_get(store: &scc_store::Store) -> crate::Result<serde_json::Value> {
    let ir = system_ir(store)?;
    let (revision, indexed_at, branch) = match store.snapshot_status()? {
        Some((snap, _)) => (snap.revision, Some(snap.indexed_at), snap.branch),
        None => ("not-indexed".to_string(), None, None),
    };
    Ok(serde_json::json!({
        "repository": store.repository(),
        "revision": revision,
        "branch": branch,
        "indexed_at": indexed_at,
        "epoch": store.model_epoch()?,
        "stats": store.stats()?,
        "files": store.all_files()?,
        "entities": ir.entities,
        "relationships": ir.relationships,
        "evidence": ir.evidence,
        "components": store.components()?,
        "flows": store.flows()?,
        "flow_graphs": store.flow_graphs()?,
        "invariants": ir.invariants,
    }))
}
