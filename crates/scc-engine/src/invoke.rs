//! Dynamic operation dispatch: `invoke(id, input_json) -> output_json`.
//!
//! The SAME fns the typed namespaces call — one implementation, two
//! interfaces (spec §4). Transports (RPC/HTTP/SDK/FFI) go through here;
//! the CLI keeps its typed calls. Outputs are the canonical types
//! serialized — never re-modelled strings.

use serde_json::Value;

// trace:exempt reason=internal-detail
pub fn invoke(
    root: &std::path::Path,
    operation: &str,
    input: Value,
) -> crate::Result<Value> {
    let store = crate::workspace::open_store(root)?;
    let config = crate::workspace::load_config(root)?;
    let stale = crate::workspace::stale_paths(&store)?;
    let engine = crate::workspace::open_engine(&store, &config, stale)?;
    let ctx = engine.context();
    let out = match operation {
        "workspace.status" => {
            let s = status_value(&store)?;
            serde_json::to_value(&s)?
        }
        "workspace.languages" => Value::String(scc_core::support_matrix_markdown()),
        "context.overview" => serde_json::to_value(ctx.overview()?)?,
        "context.atlas" => {
            let budget: Option<usize> = input.get("budget").and_then(|v| v.as_u64()).map(|v| v as usize);
            let full: bool = input.get("full").and_then(|v| v.as_bool()).unwrap_or(false);
            let unbounded: bool = input.get("unbounded").and_then(|v| v.as_bool()).unwrap_or(false);
            serde_json::to_value(ctx.atlas(budget, full, unbounded)?)?
        }
        "context.startup" => {
            let req: scc_api::StartupRequest = serde_json::from_value(input).unwrap_or(scc_api::StartupRequest { budget: None });
            let (_startup, text) = ctx.startup(&req)?;
            Value::String(text)
        }
        "context.task" => {
            let req: scc_api::TaskContextRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::task::build_task_context(&engine, &config, root, &req, None, None)?)?
        }
        "context.task_pack" => {
            let req: scc_api::TaskContextRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::task::build_enriched_task_pack(&engine, &config, root, &req, None, None)?)?
        }
        "context.component" | "context.flow" | "context.impact" | "context.verify" | "context.structural" | "surface.build" => {
            invoke_context(&ctx, &store, operation, input)?
        }
        "graph.query" => {
            let req: scc_api::QueryRequest = serde_json::from_value(input)?;
            let hit = crate::graph::query(&store, &req)?;
            serde_json::json!({
                "entities": hit.entities,
                "symbols": hit.symbols.iter().map(|(n, s, k, f)| serde_json::json!({"name": n, "signature": s, "kind": k, "file": f})).collect::<Vec<_>>(),
            })
        }
        "graph.entities" | "architecture.components" => serde_json::to_value(crate::graph::components(&store)?)?,
        "graph.flows" | "architecture.flows" => serde_json::to_value(crate::graph::flows(&store)?)?,
        "graph.relationships" => {
            let subject = input.get("subject").and_then(|v| v.as_str());
            let predicate = input.get("predicate").and_then(|v| v.as_str());
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            serde_json::to_value(crate::graph::relationships(&store, subject, predicate, limit)?)?
        }
        "index.full" => serde_json::to_value(crate::index::full(root, &config)?)?,
        "index.refresh" => {
            let req: scc_api::IndexPathsRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::index::refresh_paths(root, &config, &req.paths)?)?
        }
        "history.revisions" => serde_json::to_value(crate::history::revisions(&store)?)?,
        "history.diff" => {
            let req: scc_api::DiffRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::history::diff(&store, req.from, req.to)?)?
        }
        "export.system_ir" => {
            let req: scc_api::ExportRequest = serde_json::from_value(input).unwrap_or(scc_api::ExportRequest { format: "system-ir.json".into() });
            export_value(&store, root, &req.format)?
        }
        "architecture.drift" => serde_json::to_value(drift_value(&store)?)?,
        "integrity.invariants" | "architecture.invariants" => serde_json::to_value(invariants_value(&store)?)?,
        "integrations.list" => serde_json::to_value(crate::integrations::list(root)?)?,
        _ => return Err(crate::EngineError::Other(format!("unknown operation '{operation}' (see operations.list)"))),
    };
    Ok(out)
}

// trace:exempt reason=internal-detail
fn invoke_context(
    ctx: &crate::SccContext,
    store: &scc_store::Store,
    operation: &str,
    input: Value,
) -> crate::Result<Value> {
    let _ = store;
    Ok(match operation {
        "context.component" => {
            let req: scc_api::DetailRequest = serde_json::from_value(input)?;
            serde_json::to_value(ctx.component(&req)?)?
        }
        "context.flow" => {
            let req: scc_api::DetailRequest = serde_json::from_value(input)?;
            serde_json::to_value(ctx.flow(&req)?)?
        }
        "context.impact" => {
            let req: scc_api::ImpactRequest = serde_json::from_value(input)?;
            serde_json::to_value(ctx.impact(&req)?)?
        }
        "context.verify" => {
            let unbounded = input.get("unbounded").and_then(|v| v.as_bool()).unwrap_or(false);
            serde_json::to_value(ctx.verify(unbounded)?)?
        }
        "context.structural" => {
            let req: scc_api::StructuralRequest = serde_json::from_value(input)?;
            Value::String(ctx.structural(&req, &store.root, None)?)
        }
        "surface.build" | "ranking.important" => {
            let req: scc_api::SurfaceRequest = serde_json::from_value(input)?;
            let (result, text) = ctx.surface(&req, None)?;
            serde_json::json!({ "result": result, "text": text })
        }
        _ => unreachable!("invoke routes only context ops here"),
    })
}

// trace:exempt reason=internal-detail
fn status_value(store: &scc_store::Store) -> crate::Result<Value> {
    let repo = store.repository();
    let stale = crate::workspace::stale_paths(store)?;
    let (revision, indexed_at, branch) = match store.snapshot_status()? {
        Some((snap, _)) => (snap.revision, Some(snap.indexed_at), snap.branch),
        None => ("not-indexed".to_string(), None, None),
    };
    Ok(serde_json::json!({
        "repository": repo.name,
        "repository_id": repo.id,
        "remote": repo.url,
        "revision": revision,
        "branch": branch,
        "indexed_at": indexed_at,
        "stats": store.stats()?,
        "freshness": if stale.is_empty() { "CURRENT" } else { "STALE" },
        "stale_files": stale.iter().take(10).collect::<Vec<_>>(),
        "stale_count": stale.len(),
    }))
}

// trace:exempt reason=internal-detail
fn drift_value(store: &scc_store::Store) -> crate::Result<Value> {
    let findings = store.drift_findings(false)?;
    Ok(findings.iter().map(|(id, kind, sev, msg, at)| {
        serde_json::json!({"id": id, "kind": kind, "severity": sev, "message": msg, "created_at": at})
    }).collect())
}

// trace:exempt reason=internal-detail
fn invariants_value(store: &scc_store::Store) -> crate::Result<Value> {
    let graph = scc_graph::RealityGraph::load(store)?;
    let mut dangling = 0usize;
    for r in graph.all_rels() {
        let known = |id: &str| graph.entities.contains_key(id) || id.contains("/external_api/");
        if !known(&r.subject) || !known(&r.object) {
            dangling += 1;
        }
    }
    Ok(serde_json::json!({ "dangling_relationships": dangling, "ok": dangling == 0 }))
}

// trace:exempt reason=internal-detail
fn export_value(store: &scc_store::Store, root: &std::path::Path, format: &str) -> crate::Result<Value> {
    let ir = crate::exports::system_ir(store)?;
    match format {
        "system-ir.json" | "system-ir.jsonl" | "ccg" | "flow-graphs.json" => {
            Ok(match format {
                "system-ir.jsonl" => Value::Array(crate::exports::jsonl(&ir)?.into_iter().map(Value::String).collect()),
                "ccg" => crate::exports::ccg(&ir)?,
                "flow-graphs.json" => serde_json::to_value(store.flow_graphs()?)?,
                _ => serde_json::to_value(&ir)?,
            })
        }
        "capsule.md" => Ok(Value::String(crate::exports::capsule(root)?)),
        other => Err(crate::EngineError::Other(format!(
            "unknown export format '{other}' (use system-ir.json, system-ir.jsonl, ccg, flow-graphs.json, capsule.md)"
        ))),
    }
}
