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
        "workspace.init" => {
            let dir = crate::workspace::scc_dir(root);
            std::fs::create_dir_all(&dir)?;
            let cfg_path = crate::workspace::config_path(root);
            if !cfg_path.exists() {
                std::fs::write(&cfg_path, scc_indexer::Config::default_yaml())?;
            }
            crate::workspace::ensure_scc_ignored(root);
            serde_json::json!({"initialized": dir.to_string_lossy(), "config": cfg_path.to_string_lossy()})
        }
        "workspace.state_path" => Value::String(crate::workspace::state_dir(root).to_string_lossy().into()),
        "index.status" => {
            let s = status_value(&store)?;
            serde_json::to_value(&s)?
        }
        "resolution.run" => serde_json::to_value(crate::index::resolve_and_recompile(root)?)?,
        "graph.recompile" => {
            let r = crate::index::recompile(&store)?;
            serde_json::json!({
                "components": r.components,
                "flows": r.flows,
                "invariants": r.invariants,
                "drift": r.drift,
                "boundaries": r.boundaries,
            })
        },
        "graph.entity.get" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let all = store.all_entities()?;
            serde_json::to_value(all.into_iter().find(|e| e.id == id))?
        }
        "architecture.drift" => serde_json::to_value(crate::misc::drift(&store)?)?,
        "architecture.cochange" => {
            let min = input.get("min_commits").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
            let (pairs, enriched) = crate::misc::cochange(root, min)?;
            serde_json::json!({"pairs": pairs, "enriched": enriched})
        }
        "integrity.invariants" | "architecture.invariants" => serde_json::to_value(crate::misc::check_invariants(&store)?)?,
        "integrity.ci" => {
            let max = input.get("max_severity").and_then(|v| v.as_str()).unwrap_or("medium");
            let violations = crate::misc::check_invariants(&store)?;
            let (ok, lines) = crate::misc::ci_check(&store, &violations, max)?;
            serde_json::json!({"ok": ok, "lines": lines})
        }
        "integrations.list" => serde_json::to_value(crate::integrations::list(root)?)?,
        "integrations.doctor" => {
            let deep = input.get("deep").and_then(|v| v.as_bool()).unwrap_or(false);
            let network = input.get("network").and_then(|v| v.as_bool()).unwrap_or(false);
            serde_json::to_value(crate::integrations::doctor_report(&store, &config, root, deep, network)?)?
        }
        "lessons.add" => {
            let text = input.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let (id, path) = crate::state::lessons_add(root, text)?;
            serde_json::json!({"id": id, "path": path.to_string_lossy()})
        }
        "lessons.list" => {
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            serde_json::to_value(crate::state::lessons_list(root, limit)?)?
        }
        "beads.list" => serde_json::to_value(crate::state::beads(root, 20)?)?,
        "setup.claude" | "setup.detected" | "setup.codex" | "setup.opencode" | "setup.hermes" | "setup.omp" | "setup.pi" => {
            serde_json::json!({"ok": false, "reason": "setup operations are CLI-local (file installation); no engine state involved"})
        }
        "snapshot.save" => {
            let req: scc_api::SnapshotSaveRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::misc::snapshot_save(root, &req.task, req.budget)?)?
        }
        "snapshot.get" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(crate::snapshots::get(&store, id)?)?
        }
        "snapshot.diff" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(crate::snapshots::diff(&store, id)?)?
        }
        "checkpoint.save" => serde_json::to_value(crate::checkpoint::capture(root)?)?,
        "checkpoint.load" => serde_json::to_value(crate::checkpoint::load(root)?)?,
        "system.stitch" => {
            let members: Vec<std::path::PathBuf> = input.get("members")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(std::path::PathBuf::from)).collect())
                .unwrap_or_default();
            let (members, stitches) = crate::systems::stitch(&members)?;
            serde_json::json!({"members": members.iter().map(|m| &m.repo_id).collect::<Vec<_>>(), "stitches": stitches})
        }
        "import.scip" | "import.ccg" | "import.gitnexus" | "import.tracelayer" | "import.beads" | "import.hindsight" | "import.cbm" => {
            let format = operation.trim_start_matches("import.");
            let file = input.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let r = crate::state::import_evidence(root, format, file)?;
            serde_json::json!({
                "symbols": r.symbols,
                "calls": r.calls,
                "imports": r.imports,
                "errors": r.errors,
            })
        }
        "export.diagram" => serde_json::json!({"ok": false, "reason": "diagram rendering is CLI-local (viewer); use export.system_ir + render client-side"}),
        "runtime.ingest" => {
            let body = input.get("body").and_then(|v| v.as_str()).unwrap_or("");
            crate::state::ingest_runtime(root, body)?;
            serde_json::json!({"status": "accepted"})
        }
        "runtime.status" => serde_json::to_value(crate::state::runtime_edges(root)?)?,
        "runtime.reconcile" => serde_json::to_value(crate::state::reconcile(root)?)?,
        "ranking.important" => {
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let task = input.get("task").and_then(|v| v.as_str()).map(|s| s.to_string());
            let component = input.get("component").and_then(|v| v.as_str()).map(|s| s.to_string());
            let (entries, tasked) = ctx.important(limit, component.as_deref(), task.as_deref())?;
            serde_json::json!({"entries": entries, "tasked": tasked})
        }
        "ranking.symbols" => {
            let req: scc_api::RankRequest = serde_json::from_value(input)?;
            let ranker = engine.ranking();
            let mut ap = crate::plugins::active(root, &config);
            let hooks = ranking_hooks_from_plugins(&mut ap, req.goal.as_deref().unwrap_or(""));
            let out = ranker.symbols_with_hooks(&req, &hooks)?;
            let mut v = serde_json::to_value(&out)?;
            if !ap.diagnostics.is_empty() {
                v["plugin_diagnostics"] = serde_json::to_value(&ap.diagnostics)?;
            }
            v
        }
        "ranking.candidates" => {
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let cands = engine.ranking().candidates(goal, limit)?;
            serde_json::json!({"candidates": cands.iter().map(|c| serde_json::json!({"id": c.id, "kind": c.kind, "name": c.name, "score": c.score, "reason": c.reason})).collect::<Vec<_>>()})
        }
        "ranking.pagerank.global" => {
            let v = engine.ranking().pagerank_global()?;
            serde_json::json!({"vector": v.iter().map(|(id, s)| serde_json::json!({"id": id, "score": s})).collect::<Vec<_>>()})
        }
        "ranking.pagerank.task" => {
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let v = engine.ranking().pagerank_task(goal)?;
            serde_json::json!({"vector": v.iter().map(|(id, s)| serde_json::json!({"id": id, "score": s})).collect::<Vec<_>>()})
        }
        "ranking.final_importance" => {
            let f = |k: &str| input.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let has_task = input.get("has_task").and_then(|v| v.as_bool()).unwrap_or(true);
            let s = scc_context::pagerank::final_importance(f("task_ppr"), f("global_ppr"), f("lexical"), f("semantic"), f("confidence"), f("criticality"), f("change_risk"), f("novelty"), has_task);
            serde_json::json!({"score": s})
        }
        "ranking.edge_weight" => {
            let predicate = input.get("predicate").and_then(|v| v.as_str()).unwrap_or("calls");
            let confidence = input.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let total = input.get("total_symbols").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let indeg = input.get("target_in_degree").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            serde_json::json!({"weight": scc_context::pagerank::edge_weight(predicate, scc_core::Provenance::Extracted, confidence, total, indeg)})
        }
        "ranking.architectural_specificity" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let exported = input.get("exported").and_then(|v| v.as_bool()).unwrap_or(false);
            serde_json::json!({"specificity": if exported { 1.15 } else { 1.0 }, "id": id})
        }
        "ranking.explain" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let req = scc_api::RankRequest { goal: Some(goal.into()), limit: 1000, explain: true, include_features: true, include_intermediate: true };
            let ranker = engine.ranking();
            let mut ap = crate::plugins::active(root, &config);
            let hooks = ranking_hooks_from_plugins(&mut ap, goal);
            let out = ranker.symbols_with_hooks(&req, &hooks)?;
            match out.items.into_iter().find(|i| i.id == id) {
                Some(item) => serde_json::to_value(&item)?,
                None => serde_json::json!({"error": format!("symbol {id} not in ranking")}),
            }
        }
        "ranking.global" | "ranking.task" | "ranking.entities" => {
            let goal = input.get("goal").and_then(|v| v.as_str()).map(|s| s.to_string());
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let req = scc_api::RankRequest { goal, limit, explain: false, include_features: true, include_intermediate: false };
            serde_json::to_value(engine.ranking().symbols(&req)?)?
        }
        "selection.mmr" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let ranked: Vec<(String, f64)> = req.ranked.iter().map(|e| (e.id.clone(), e.value)).collect();
            // Similarity hook op: ranking.similarity.<plugin> — default: same-component/path grouping is caller-side.
            let out = crate::ranking::mmr_select(&ranked, &|_a: &str, _b: &str| 0.0, req.lambda.unwrap_or(0.5), ranked.len());
            serde_json::json!({"selected": out})
        }
        "selection.quotas" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let rows: Vec<(String, String, f64, usize)> = req.ranked.iter().map(|e| (e.id.clone(), e.kind.clone(), e.value, e.token_cost)).collect();
            let quotas: Vec<(String, f64)> = req.quotas.unwrap_or_default().iter().map(|q| (q.kind.clone(), q.fraction)).collect();
            let budget: usize = req.ranked.iter().map(|e| e.token_cost).sum();
            serde_json::json!({"selected": crate::ranking::apply_quotas(&rows, &quotas, budget)})
        }
        "selection.budget" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let items: Vec<scc_core::ContextItem> = req.ranked.iter().map(|e| scc_core::ContextItem { id: e.id.clone(), value: e.value, token_cost: e.token_cost, required: false, group: e.group.clone() }).collect();
            let budget: usize = items.iter().map(|i| i.token_cost).sum();
            serde_json::json!({"selected": crate::ranking::select_with_budget(&items, budget, budget)})
        }
        "plugins.list" => {
            let ap = crate::plugins::active(root, &config);
            serde_json::json!({"plugins": scc_plugin_host::discover(root).iter().map(|p| &p.manifest.id).collect::<Vec<_>>(), "lock": crate::plugins::lock_entries(&ap)})
        }
        "plugins.describe" => {
            let ap = crate::plugins::active(root, &config);
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match ap.plugins.iter().find(|p| p.manifest.id == id) {
                Some(p) => serde_json::json!({"manifest": {"id": p.manifest.id, "name": p.manifest.name, "version": p.manifest.version, "api": p.manifest.api, "operations": p.manifest.operations, "timeout_ms": p.manifest.timeout_ms, "failure_policy": p.manifest.failure_policy, "deterministic": p.manifest.deterministic}, "lock": scc_plugin_host::lock_entry(p)}),
                None => serde_json::json!({"error": format!("unknown plugin '{id}'")}),
            }
        }
        "plugins.doctor" => {
            let ap = crate::plugins::active(root, &config);
            serde_json::json!({"plugins": ap.plugins.iter().map(|p| serde_json::json!({"id": p.manifest.id, "operations": p.manifest.operations})).collect::<Vec<_>>(), "diagnostics": ap.diagnostics})
        }
        "plugins.invoke" => {
            let op = input.get("operation").and_then(|v| v.as_str()).unwrap_or(operation);
            let inner = input.get("input").cloned().unwrap_or(serde_json::json!({}));
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::call_operation(&mut ap, op, inner)?
        }
        _ => {
            // Custom plugin operations: no per-transport code needed (spec 31).
            let mut ap = crate::plugins::active(root, &config);
            match scc_plugin_host::provider_for(&ap.plugins, operation) {
                Ok(_) => crate::plugins::call_operation(&mut ap, operation, input)?,
                Err(_) => return Err(crate::EngineError::Other(format!("unknown operation '{operation}' (see operations.list)"))),
            }
        }
    };
    Ok(out)
}

// trace:exempt reason=internal-detail
fn ranking_hooks_from_plugins(
    ap: &mut crate::plugins::ActivePlugins,
    goal: &str,
) -> crate::ranking::RankHooks {
    use std::sync::Arc;
    let mut hooks = crate::ranking::RankHooks::default();
    // Snapshot what each plugin provides (borrow ends before mutation).
    let specs: Vec<(String, String, Vec<String>)> = ap.plugins.iter()
        .map(|p| (p.manifest.id.clone(), p.manifest.version.clone(), p.manifest.operations.clone()))
        .collect();
    for (pid, ver, ops) in specs {
        let plug = match ap.plugins.iter().find(|p| p.manifest.id == pid).cloned() {
            Some(p) => Arc::new(p),
            None => continue,
        };
        if ops.iter().any(|o| o == "ranking.seed") {
            let plug = Arc::clone(&plug);
            let g = goal.to_string();
            hooks.seed_providers.push(Box::new(move |_goal| {
                let input = serde_json::json!({"goal": g});
                match scc_plugin_host::call(&plug, "ranking.seed", input, None) {
                    Ok(v) => v.get("seeds").and_then(|s| s.as_array()).map(|a| {
                        a.iter().map(|e| scc_core::TaskSeed {
                            kind: e.get("kind").and_then(|x| x.as_str()).unwrap_or("symbol").into(),
                            id: e.get("id").and_then(|x| x.as_str()).unwrap_or("").into(),
                            weight: e.get("weight").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        }).filter(|s| !s.id.is_empty()).collect()
                    }).unwrap_or_default(),
                    Err(_) => Vec::new(),
                }
            }));
        }
        if ops.iter().any(|o| o == "ranking.feature") {
            let plug = Arc::clone(&plug);
            hooks.features.push(Box::new(move |sym, goal| {
                let input = serde_json::json!({"symbol": sym, "goal": goal});
                match scc_plugin_host::call(&plug, "ranking.feature", input, None) {
                    Ok(v) => crate::ranking::RankFeatureValue {
                        name: format!("{pid}.feature"),
                        score: v.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        weight: v.get("weight").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        reason: v.get("reason").and_then(|x| x.as_str()).unwrap_or("").into(),
                    },
                    Err(_) => crate::ranking::RankFeatureValue { name: format!("{pid}.feature"), score: 0.0, weight: 0.0, reason: String::new() },
                }
            }));
        }
        if ops.iter().any(|o| o == "ranking.rerank") {
            let plug = Arc::clone(&plug);
            hooks.rerankers.push(Box::new(move |items, goal| {
                let input = serde_json::json!({"goal": goal, "items": items.iter().map(|i: &scc_api::RankItem| i.id.clone()).collect::<Vec<_>>()});
                if let Ok(v) = scc_plugin_host::call(&plug, "ranking.rerank", input, None) {
                    if let Some(order) = v.get("order").and_then(|o| o.as_array()) {
                        let pos: std::collections::HashMap<&str, usize> =
                            order.iter().enumerate().filter_map(|(i, x)| x.as_str().map(|s| (s, i))).collect();
                        items.sort_by_key(|i| (pos.get(i.id.as_str()).copied().unwrap_or(usize::MAX), i.id.clone()));
                        for (k, it) in items.iter_mut().enumerate() { it.position = k + 1; }
                    }
                }
            }));
        }
        let _ = ver;
    }
    hooks
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
// trace:exempt reason=internal-detail
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
