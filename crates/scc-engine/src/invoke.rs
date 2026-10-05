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
            let model: bool = input.get("model").and_then(|v| v.as_bool()).unwrap_or(false);
            // §74: `model: true` returns the structured SystemAtlas
            // alongside the pack — one derivation, two views.
            if model {
                let scope = if full {
                    scc_context::atlas::AtlasScope::Full
                } else {
                    scc_context::atlas::AtlasScope::Production
                };
                let pack = ctx.atlas(budget, full, unbounded)?;
                serde_json::json!({"pack": pack, "model": ctx.atlas_model(scope)?})
            } else {
                serde_json::to_value(ctx.atlas(budget, full, unbounded)?)?
            }
        }
        "context.startup" => {
            let want_model = input.get("model").and_then(|v| v.as_bool()).unwrap_or(false);
            let req: scc_api::StartupRequest = serde_json::from_value(input).unwrap_or(scc_api::StartupRequest { budget: None });
            // Structured triple: text + budget + artifact. Transports that
            // need a bare string take `.text`; nothing re-derives startup.
            let (startup, text) = ctx.startup(&req)?;
            let budget = scc_context::startup::allocate_startup_budget(&ctx.engine.ctx(), req.budget);
            // Plugin startup sections (§124 item 29): verbatim markdown
            // under provenance headers, appended by the engine so every
            // transport delivers them. Skips are recorded inline (startup
            // has no warnings channel); symbol visibility unaffected.
            let (sections, notes) = crate::plugins::startup_sections(root, &config);
            let mut text = text;
            text.push_str(&sections);
            for n in notes {
                text.push_str(&format!("\n(startup section skipped: {n})\n"));
            }
            // §75: `model: true` adds the structured decomposition
            // (atlas/skeleton/surface/coverage/omissions by key).
            let mut out = serde_json::json!({"text": text, "budget": budget, "artifact": startup.artifact});
            if want_model {
                out["model"] = serde_json::to_value(ctx.startup_model(&req)?)?;
            }
            out
        }
        "context.task" => {
            let req: scc_api::TaskContextRequest = serde_json::from_value(input)?;
            let (scorer, reranker) = crate::inference::rankers(&store, &config, &req.goal);
            let scorer_trait: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let reranker_trait: Option<&dyn scc_context::rank::Reranker> =
                reranker.as_ref().map(|r| r as &dyn scc_context::rank::Reranker);
            serde_json::to_value(crate::task::build_task_context(&engine, &config, root, &req, scorer_trait, reranker_trait)?)?
        }
        "context.task_pack" => {
            let req: scc_api::TaskContextRequest = serde_json::from_value(input)?;
            let (scorer, reranker) = crate::inference::rankers(&store, &config, &req.goal);
            let scorer_trait: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let reranker_trait: Option<&dyn scc_context::rank::Reranker> =
                reranker.as_ref().map(|r| r as &dyn scc_context::rank::Reranker);
            serde_json::to_value(crate::task::build_enriched_task_pack(&engine, &config, root, &req, scorer_trait, reranker_trait)?)?
        }
        "context.task_delta" => {
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let budget: usize = input.get("budget").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(scc_core::ContextBudget::default().task_delta);
            // Spec §77: inspection without display must not suppress future
            // deltas. record_visibility=false skips the ledger write;
            // default true preserves current CLI behavior.
            let record = input.get("record_visibility").and_then(|v| v.as_bool()).unwrap_or(true);
            let (scorer, _) = crate::inference::rankers(&store, &config, goal);
            let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let (delta, ids) = ctx.task_delta(goal, budget, semantic)?;
            if record {
                ctx.record_task_delta_ids(&ids);
            }
            serde_json::json!({"delta": delta, "delta_ids": ids})
        }
        "context.subagent" => {
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let files: Vec<String> = input.get("files").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            let symbols: Vec<String> = input.get("symbols").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            let budget = input.get("budget").and_then(|v| v.as_u64()).map(|b| b as usize);
            serde_json::to_value(ctx.subagent(goal, &files, &symbols, budget)?)?
        }
        "context.compress" => {
            // Pack-only compression ladder, no external summarizer, no delta,
            // no ledger: same derivation the CLI uses with --cmd omitted.
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let budget = input.get("budget").and_then(|v| v.as_u64()).map(|b| b as usize);
            let (scorer, reranker) = crate::inference::rankers(&store, &config, goal);
            let scorer_trait: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let reranker_trait: Option<&dyn scc_context::rank::Reranker> =
                reranker.as_ref().map(|r| r as &dyn scc_context::rank::Reranker);
            let pack = crate::task::build_enriched_task_pack(&engine, &config, root, &scc_api::TaskContextRequest { goal: goal.into(), files: vec![], symbols: vec![], budget, hook: false, record_visibility: true }, scorer_trait, reranker_trait)?;
            serde_json::to_value(pack)?
        }
        "context.component" | "context.flow" | "context.impact" | "context.verify" | "context.structural" | "source.structural" | "surface.build" | "surface.compile" | "surface.global" | "surface.task" | "surface.render" => {
            invoke_context(&ctx, &store, &config, operation, input)?
        }
        "graph.query" => {
            let req: scc_api::QueryRequest = serde_json::from_value(input)?;
            let hit = crate::graph::query(&store, &req)?;
            serde_json::json!({
                "entities": hit.entities,
                "symbols": hit.symbols.iter().map(|(n, s, k, f, l)| serde_json::json!({"name": n, "signature": s, "kind": k, "file": f, "line": l})).collect::<Vec<_>>(),
            })
        }
        "graph.traverse" => {
            let req: scc_api::TraverseRequest = serde_json::from_value(input)?;
            let cc = ctx.engine.ctx();
            let (entities, relationships) = crate::graph::traverse(&cc, &req)?;
            serde_json::json!({"entities": entities, "relationships": relationships, "trusted_only": req.trusted_only})
        }
        "graph.entities" | "architecture.components" => serde_json::to_value(crate::graph::components(&store)?)?,
        "graph.flows" | "architecture.flows" => serde_json::to_value(crate::graph::flows(&store)?)?,
        "graph.relationships" => {
            let subject = input.get("subject").and_then(|v| v.as_str());
            let predicate = input.get("predicate").and_then(|v| v.as_str());
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            serde_json::to_value(crate::graph::relationships(&store, subject, predicate, limit)?)?
        }
        "graph.search" => {
            let q = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let mut entities = store.search_entities(q, limit)?;
            if entities.is_empty() { entities = store.search_entities_like(q, limit)?; }
            serde_json::to_value(entities)?
        }
        "graph.search_symbols" => {
            let q = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let mut symbols = store.search_symbols(q, limit)?;
            if symbols.is_empty() { symbols = store.search_symbols_like(q, limit)?; }
            serde_json::json!({"symbols": symbols.iter().map(|(n, s, k, f, l)| serde_json::json!({"name": n, "signature": s, "kind": k, "file": f, "line": l})).collect::<Vec<_>>()})
        }
        "evidence.get" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(store.get_evidence(id)?)?
        }
        "evidence.list" | "evidence.search" => {
            let path = input.get("path").and_then(|v| v.as_str());
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
            let mut out = match path {
                Some(p) => store.evidence_for_path(p)?,
                None => store.all_evidence()?,
            };
            out.truncate(limit.max(1));
            serde_json::to_value(out)?
        }
        "runtime.signatures" => {
            let rows = store.trace_signatures()?;
            serde_json::json!({"signatures": rows.iter().map(|(sig, count, lat, err, last)| serde_json::json!({"signature": sig, "count": count, "latency_ms": lat, "errors": err, "last_observed": last})).collect::<Vec<_>>()})
        }
        "integrations.describe" => {
            let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let all = crate::integrations::list(root)?;
            match all.into_iter().find(|(n, _)| n == name) {
                Some((n, scope)) => serde_json::json!({"name": n, "scope": scope}),
                None => serde_json::json!({"error": format!("unknown integration '{name}'")}),
            }
        }
        "index.full" => serde_json::to_value(crate::index::full(root, &config)?)?,
        "index.refresh" | "index.paths" => {
            let req: scc_api::IndexPathsRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::index::refresh_paths(root, &config, &req.paths)?)?
        }
        "history.revisions" | "history.list" => serde_json::to_value(crate::history::revisions(&store)?)?,
        "history.diff" => {
            let req: scc_api::DiffRequest = serde_json::from_value(input)?;
            serde_json::to_value(crate::history::diff(&store, req.from, req.to)?)?
        }
        "model.get" | "model.components" | "model.flows" | "model.invariants" => serde_json::to_value(crate::exports::model_get(&store)?)?,
        "context.external_docs" | "context.docs" => {
            let dep = input.get("dependency").and_then(|v| v.as_str()).unwrap_or("");
            Value::String(crate::state::external_docs(root, dep)?)
        }
        "embeddings.build" => crate::state::embeddings_build(root)?,
        "embeddings.get" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            crate::state::embeddings_get(&store, id)?
        }
        "embeddings.status" => crate::state::embeddings_status(&store)?,
        "export.system_ir" => {
            let req: scc_api::ExportRequest = serde_json::from_value(input).unwrap_or(scc_api::ExportRequest { format: "system-ir.json".into() });
            export_value(&store, root, &req.format)?
        }
        "export.system_ir_jsonl" => export_value(&store, root, "system-ir.jsonl")?,
        "export.ccg" => export_value(&store, root, "ccg")?,
        "export.flow_graphs" => export_value(&store, root, "flow-graphs.json")?,
        "export.snap" | "export.capsule" => export_value(&store, root, "capsule.md")?,
        "operations.list" => {
            serde_json::json!({"operations": crate::ops::ids(), "api_version": scc_api::API_VERSION})
        }
        "operations.capabilities" => {
            serde_json::to_value(crate::ops::capabilities())?
        }
        "operations.schema" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(crate::ops::input_schema(id).ok_or_else(|| {
                crate::EngineError::Other(format!("unknown operation '{id}' (see operations.list)"))
            })?)?
        }
        "operations.describe" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match crate::ops::describe(id) {
                Some(d) => serde_json::to_value(d)?,
                None => return Err(crate::EngineError::Other(format!("unknown operation '{id}' (see operations.list)"))),
            }
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
        "workspace.scan" => {
            let path = input.get("path").and_then(|v| v.as_str());
            serde_json::to_value(crate::status::scan(root, &config, path)?)?
        }
        "workspace.session" => serde_json::to_value(crate::workspace::open_session(&store, &config)?)?,
        "workspace.session_check" => {
            let session: crate::workspace::Session = serde_json::from_value(input.get("session").cloned().unwrap_or(serde_json::Value::Null))?;
            serde_json::json!({"current": crate::workspace::session_is_current(&store, &config, &session)?})
        }
        "index.status" => {
            let s = status_value(&store)?;
            serde_json::to_value(&s)?
        }
        "resolution.run" => serde_json::to_value(crate::index::resolve_and_recompile(root)?)?,
        "graph.recompile" => {
            let r = crate::index::recompile_with_signals(root, &config, &store)?;
            serde_json::json!({
                "components": r.components,
                "flows": r.flows,
                "invariants": r.invariants,
                "drift": r.drift,
                "boundaries": r.boundaries,
            })
        },
        "graph.explain" => {
            let subject = input.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let predicate = input.get("predicate").and_then(|v| v.as_str()).unwrap_or("");
            let object = input.get("object").and_then(|v| v.as_str()).unwrap_or("");
            let cc = ctx.engine.ctx();
            crate::graph::explain(&cc, subject, predicate, object)
        }
        "graph.entity.get" | "graph.entity" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let found = store.all_entities()?.into_iter().find(|e| e.id == id);
            match found {
                None => serde_json::json!({"entity": null, "trusted": false, "reason": "unknown id"}),
                Some(e) => {
                    let cc = ctx.engine.ctx();
                    match cc.view.entity(&e.id) {
                        Some(_) => serde_json::json!({"entity": e, "trusted": true, "reason": null}),
                        None => serde_json::json!({"entity": e, "trusted": false, "reason": "hidden by TrustedGraphView (stale evidence or below trust floor)"}),
                    }
                }
            }
        }
        "architecture.drift" | "model.drift" => serde_json::to_value(crate::misc::drift(&store)?)?,
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
        "integrations.list" | "integration.list" => serde_json::to_value(crate::integrations::list(root)?)?,
        "integrations.doctor" | "integration.doctor" => {
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
        "beads.list" | "beads.active" => serde_json::to_value(crate::state::beads(root, 20)?)?,
        "viewer.panels" => {
            // Plugin viewer panels (§124 item 32): structured data the CLI
            // renders into viewer pages — same provenance pattern as the
            // other contribution points, one seam every transport inherits.
            let (panels, notes) = crate::plugins::viewer_panels(root, &config);
            serde_json::json!({"panels": panels, "skipped": notes})
        }
        "viewer.snapshot" => {
            return Err(crate::EngineError::Other("viewer.snapshot is CLI-local browser capture; not an engine operation".into()));
        }
        "setup.claude" | "setup.detected" | "setup.codex" | "setup.opencode" | "setup.hermes" | "setup.omp" | "setup.pi" => {
            return Err(crate::EngineError::Other("setup operations are CLI-local file installation (harness dirs, home directory); not engine operations".into()));
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
        _ if operation.starts_with("import.") => {
            // Plugin evidence providers (spec §31): `import.<plugin-id>`
            // commits the plugin's batch through validate+commit.
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
        "export.diagram" | "diagram.render" => {
            let format = input.get("format").and_then(|v| v.as_str()).unwrap_or("mermaid");
            crate::exports::diagram(&store, format)?
        }
        "index.watch" => {
            return Err(crate::EngineError::Other("index.watch is a streaming filesystem watch, not a request/response operation; run `scc watch`".into()));
        }
        "runtime.ingest" => {
            let body = input.get("body").and_then(|v| v.as_str()).unwrap_or("");
            crate::state::ingest_runtime(root, body)?;
            serde_json::json!({"status": "accepted"})
        }
        "runtime.status" => serde_json::to_value(crate::state::runtime_edges(root)?)?,
        "runtime.reconcile" => serde_json::to_value(crate::state::reconcile(root)?)?,
        "ranking.important" | "surface.important" => {
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let task = input.get("task").and_then(|v| v.as_str()).map(|s| s.to_string());
            let component = input.get("component").and_then(|v| v.as_str()).map(|s| s.to_string());
            let (entries, tasked) = ctx.important(limit, component.as_deref(), task.as_deref())?;
            serde_json::json!({"entries": entries, "tasked": tasked})
        }
        "ranking.symbols" | "surface.rank" => {
            let req: scc_api::RankRequest = serde_json::from_value(input)?;
            let ranker = engine.ranking();
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
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
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, goal);
            let cands = engine.ranking().candidates_with(goal, limit, &hooks)?;
            serde_json::json!({"candidates": cands.iter().map(|c| serde_json::json!({"id": c.id, "kind": c.kind, "name": c.name, "score": c.score, "reason": c.reason})).collect::<Vec<_>>()})
        }
        "ranking.project_symbols" => {
            // Projection introspection (§123 intermediate): map a
            // universe vector to per-symbol scores. `vector`: explicit
            // (id, score) rows; or `source`: "global" / task `goal`.
            let rows: Vec<(String, f64)> = match input.get("vector").and_then(|v| v.as_array()) {
                Some(arr) => arr.iter().map(|r| (
                    r.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    r.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0),
                )).collect(),
                None => match input.get("source").and_then(|v| v.as_str()) {
                    Some("global") => engine.ranking().pagerank_global()?,
                    _ => {
                        let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
                        engine.ranking().pagerank_task(goal)?
                    }
                },
            };
            let out = engine.ranking().project_symbols(&rows)?;
            serde_json::json!({"symbols": out.iter().map(|(id, s)| serde_json::json!({"id": id, "score": s})).collect::<Vec<_>>()})
        }
        "ranking.features" => {
            // Feature-score introspection (§123.13): per-symbol core +
            // plugin feature decomposition before the blend. Same hooks
            // as ranking.symbols; no new math, projection only.
            let req: scc_api::RankRequest = serde_json::from_value(input)?;
            let ranker = engine.ranking();
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, req.goal.as_deref().unwrap_or(""));
            let out = ranker.symbols_with_hooks(&req, &hooks)?;
            serde_json::json!({"features": out.items.iter().map(|i| serde_json::json!({
                "id": i.id, "position": i.position,
                "task_ppr": i.features.task_ppr, "global_ppr": i.features.global_ppr,
                "lexical": i.features.lexical, "semantic": i.features.semantic,
                "confidence": i.features.confidence, "criticality": i.features.criticality,
                "change_risk": i.features.change_risk, "novelty": i.features.novelty,
                "specificity": i.specificity, "plugin_features": i.plugin_features,
            })).collect::<Vec<_>>()})
        }
        "ranking.reference_graph" => {
            let edges = engine.ranking().reference_graph()?;
            serde_json::to_value(&edges)?
        }
        "ranking.universe" => {
            let nodes = engine.ranking().universe()?;
            serde_json::json!({"nodes": nodes.iter().map(|(id, k)| serde_json::json!({"id": id, "kind": k})).collect::<Vec<_>>()})
        }
        "ranking.edges" => {
            let edges = engine.ranking().rank_edges()?;
            serde_json::json!({"edges": edges.iter().map(|(s, p, o, w)| serde_json::json!({"subject": s, "predicate": p, "object": o, "weight": w})).collect::<Vec<_>>()})
        }
        "ranking.seeds" => {
            // Task-seed introspection (§123.11): lexical seeds merged with
            // plugin seed providers (weight sums by id). Read-only stage
            // view — the same merge symbols_with_hooks consumes.
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, goal);
            let seeds = engine.ranking().seeds_with(goal, &hooks)?;
            serde_json::json!({"seeds": seeds.iter().map(|x| serde_json::json!({"id": x.id, "kind": x.kind, "weight": x.weight})).collect::<Vec<_>>()})
        }
        "ranking.pagerank.global" | "ranking.global_vector" => {
            // Raw stage introspection: no plugin hooks by contract.
            let v = engine.ranking().pagerank_global()?;
            serde_json::json!({"vector": v.iter().map(|(id, s)| serde_json::json!({"id": id, "score": s})).collect::<Vec<_>>()})
        }
        "ranking.pagerank.task" | "ranking.task_vector" => {
            // Raw stage introspection: no plugin hooks by contract.
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
        "ranking.score_entries" => {
            let rows: Vec<crate::ranking::ScoreRow> = input.get("entries")
                .and_then(|v| v.as_array()).map(|arr| arr.iter().map(|e| {
                    let f = |k: &str| e.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    crate::ranking::ScoreRow {
                        id: e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        task_ppr: f("task_ppr"), global_ppr: f("global_ppr"),
                        lexical: f("lexical"), semantic: f("semantic"),
                        confidence: f("confidence"), criticality: f("criticality"),
                        change_risk: f("change_risk"), novelty: f("novelty"),
                        has_task: e.get("has_task").and_then(|v| v.as_bool()).unwrap_or(true),
                    }
                }).collect()).unwrap_or_default();
            let out = crate::ranking::score_entries(&rows);
            serde_json::json!({"scores": out.iter().map(|(id, s)| serde_json::json!({"id": id, "score": s})).collect::<Vec<_>>()})
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
        "ranking.entry" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match ctx.surface_entry(id)? {
                Some(e) => serde_json::to_value(&e)?,
                None => serde_json::json!({"error": format!("entry {id} not in surface map")}),
            }
        }
        "ranking.trace" => {
            // Full ranking trace (§19): items + the seed/required
            // inputs the blend consumed. Same computation as
            // ranking.symbols; the envelope is the audit path.
            let req: scc_api::RankRequest = serde_json::from_value(input)?;
            let goal = req.goal.clone().unwrap_or_default();
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, &goal);
            let (out, seeds, required) = engine.ranking().trace_with_hooks(&req, &hooks)?;
            let mut v = serde_json::to_value(&out)?;
            v["seeds"] = serde_json::json!(seeds);
            v["required"] = serde_json::json!(required);
            v
        }
        "ranking.explain" | "surface.explain" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let req = scc_api::RankRequest { profile: None, goal: Some(goal.into()), limit: 1000, explain: true, include_features: true, include_intermediate: true };
            let ranker = engine.ranking();
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
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
            let req = scc_api::RankRequest { profile: None, goal, limit, explain: false, include_features: true, include_intermediate: false };
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, req.goal.as_deref().unwrap_or(""));
            serde_json::to_value(engine.ranking().symbols_with_hooks(&req, &hooks)?)?
        }
        "selection.mmr" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let ranked: Vec<(String, f64)> = req.ranked.iter().map(|e| (e.id.clone(), e.value)).collect();
            let groups: Vec<Option<String>> = req.ranked.iter().map(|e| e.group.clone()).collect();
            // Similarity hook chain (DoD 27): `similarity` extensions fire
            // first; the same-group default runs last. Groups ride the
            // request's `group` field (component/path supplied by caller).
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            // Diversity-policy replacement (§124 item 24): a single
            // declarer replaces MMR wholesale (verbatim answer).
            if let Some(sel) = crate::plugins::diversity_selection(&ap, &req)? {
                let mut out = serde_json::json!({"selected": sel});
                if !ap.diagnostics.is_empty() { out["plugin_diagnostics"] = serde_json::to_value(&ap.diagnostics)?; }
                return Ok(out);
            }
            let hooks = ranking_hooks_from_plugins(&mut ap, "");
            let sims = std::sync::Arc::new(hooks.similarities);
            let out = scc_context::selector::mmr_diversify(
                &ranked,
                |a: &str, b: &str| {
                    let (ga, gb) = (crate::ranking::group_of2(&ranked, &groups, a), crate::ranking::group_of2(&ranked, &groups, b));
                    crate::ranking::fold_similarity(&sims, a, b, ga, gb)
                },
                req.lambda.unwrap_or(0.5),
                ranked.len(),
            );
            serde_json::json!({"selected": out})
        }
        "selection.quotas" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let rows: Vec<(String, String, f64, usize)> = req.ranked.iter().map(|e| (e.id.clone(), e.kind.clone(), e.value, e.token_cost)).collect();
            let mut quotas: Vec<(String, f64)> = req.quotas.clone().unwrap_or_default().iter().map(|q| (q.kind.clone(), q.fraction)).collect();
            // Quota-policy extensions (§124 item 25): declaring plugins
            // override per-kind fractions (plugin wins per kind). Merged
            // before the single apply_quotas call — one math path.
            let mut ap = crate::plugins::active(root, &config);
            let (overrides, notes) = crate::plugins::quota_overrides(&ap, &req);
            for (kind, frac) in &overrides {
                if let Some(slot) = quotas.iter_mut().find(|(k, _)| k == kind) { slot.1 = *frac; }
                else { quotas.push((kind.clone(), *frac)); }
            }
            if !notes.is_empty() { ap.diagnostics.extend(notes); }
            let budget: usize = req.ranked.iter().map(|e| e.token_cost).sum();
            let mut out = serde_json::json!({"selected": crate::ranking::apply_quotas(&rows, &quotas, budget)});
            if !overrides.is_empty() {
                out["quota_overrides"] = serde_json::json!(overrides.iter().map(|(k, f)| serde_json::json!({"kind": k, "fraction": f})).collect::<Vec<_>>());
            }
            if !ap.diagnostics.is_empty() { out["plugin_diagnostics"] = serde_json::to_value(&ap.diagnostics)?; }
            out
        }
        "selection.required" => {
            // Required-coverage set (§123: never-omit entries): engine
            // required_ids + plugin coverage providers, unioned. Same
            // inputs symbols_with_hooks blends criticality from.
            let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let mut ap = crate::plugins::active(root, &config);
            crate::plugins::order_extensions(&crate::plugins::collect_extensions(&ap))?;
            let hooks = ranking_hooks_from_plugins(&mut ap, goal);
            let base: usize = engine.ranking().required_with("", &crate::ranking::RankHooks::default())?.len();
            let mut ids: Vec<String> = engine.ranking().required_with(goal, &hooks)?.into_iter().collect();
            ids.sort();
            let contributed = ids.len().saturating_sub(base);
            serde_json::json!({"required": ids, "plugin_contributed": contributed})
        }
        "selection.preview" => {
            // Selection-effects introspection (§123.14): per-stage
            // survivors through the DEFAULT chain (MMR → quotas →
            // budget) over caller-supplied rows. Default math only, no
            // plugin policies — shows WHERE each id drops out.
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let ranked: Vec<(String, f64)> = req.ranked.iter().map(|e| (e.id.clone(), e.value)).collect();
            let groups: Vec<Option<String>> = req.ranked.iter().map(|e| e.group.clone()).collect();
            let sim = |a: &str, b: &str| {
                let (ga, gb) = (crate::ranking::group_of2(&ranked, &groups, a), crate::ranking::group_of2(&ranked, &groups, b));
                crate::ranking::default_similarity(ga, gb)
            };
            let after_mmr = scc_context::selector::mmr_diversify(
                &ranked, sim, req.lambda.unwrap_or(0.5), ranked.len());
            let rows: Vec<(String, String, f64, usize)> = req.ranked.iter()
                .map(|e| (e.id.clone(), e.kind.clone(), e.value, e.token_cost)).collect();
            let quotas: Vec<(String, f64)> = req.quotas.unwrap_or_default().iter()
                .map(|q| (q.kind.clone(), q.fraction)).collect();
            let budget: usize = req.ranked.iter().map(|e| e.token_cost).sum();
            let after_quotas = crate::ranking::apply_quotas(&rows, &quotas, budget);
            let qset: std::collections::BTreeSet<&str> =
                after_quotas.iter().map(|x| x.as_str()).collect();
            let items: Vec<scc_core::ContextItem> = req.ranked.iter()
                .filter(|e| qset.contains(e.id.as_str()))
                .map(|e| scc_core::ContextItem { id: e.id.clone(), value: e.value,
                    token_cost: e.token_cost, required: false, group: e.group.clone() }).collect();
            let keep = crate::ranking::select_with_budget(&items, budget, budget);
            let after_budget: Vec<String> = keep.into_iter().map(|i| items[i].id.clone()).collect();
            serde_json::json!({
                "after_mmr": after_mmr, "after_quotas": after_quotas,
                "after_budget": after_budget,
            })
        }
        "selection.budget" | "selection.optimize" | "surface.select" => {
            let req: scc_api::SelectionRequest = serde_json::from_value(input)?;
            let items: Vec<scc_core::ContextItem> = req.ranked.iter().map(|e| scc_core::ContextItem { id: e.id.clone(), value: e.value, token_cost: e.token_cost, required: false, group: e.group.clone() }).collect();
            let budget: usize = items.iter().map(|i| i.token_cost).sum();
            // Budget-optimizer extension (§124 item 27): at most one
            // declarer; the plugin returns the full selected id list.
            let ap = crate::plugins::active(root, &config);
            match crate::plugins::budget_selection(&ap, &req, &items, budget)? {
                Some(sel) => {
                    let mut out = serde_json::json!({"selected": sel});
                    if !ap.diagnostics.is_empty() { out["plugin_diagnostics"] = serde_json::to_value(&ap.diagnostics)?; }
                    out
                }
                None => {
                    let keep = crate::ranking::select_with_budget(&items, budget, budget);
                    let ids: Vec<&str> = keep.into_iter().map(|i| items[i].id.as_str()).collect();
                    serde_json::json!({"selected": ids})
                }
            }
        }
        "plugins.list" => {
            let ap = crate::plugins::active(root, &config);
            serde_json::json!({"plugins": scc_plugin_host::discover(root).iter().map(|p| &p.manifest.id).collect::<Vec<_>>(), "lock": crate::plugins::lock_entries(&ap)})
        }
        "plugins.describe" | "plugins.inspect" => {
            let ap = crate::plugins::active(root, &config);
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match ap.plugins.iter().find(|p| p.manifest.id == id) {
                Some(p) => serde_json::json!({"manifest": {"id": p.manifest.id, "name": p.manifest.name, "version": p.manifest.version, "api": p.manifest.api, "operations": p.manifest.operations, "timeout_ms": p.manifest.timeout_ms, "failure_policy": p.manifest.failure_policy, "deterministic": p.manifest.deterministic, "runtime": format!("{:?}", p.manifest.runtime).to_lowercase(), "extensions": p.manifest.extensions}, "lock": scc_plugin_host::lock_entry(p)}),
                None => serde_json::json!({"error": format!("unknown plugin '{id}'")}),
            }
        }
        "plugins.doctor" => {
            let ap = crate::plugins::active(root, &config);
            serde_json::json!({"plugins": ap.plugins.iter().map(|p| serde_json::json!({"id": p.manifest.id, "operations": p.manifest.operations})).collect::<Vec<_>>(), "diagnostics": ap.diagnostics})
        }
        "plugins.contribute" => {
            let plugin = input.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
            let batch = input.get("batch").cloned().unwrap_or(serde_json::json!({}));
            if plugin.is_empty() {
                return Err(crate::EngineError::Other("plugins.contribute requires a `plugin` id".into()));
            }
            crate::plugins::commit_contribution(&store, plugin, &batch)?
        }
        "plugins.promote" => {
            // Sidecar promotion (§124 item 36): selected sidecar findings
            // enter the canonical graph through the normal validate+commit
            // path. Same provenance/conflict rules as every contribution.
            let plugin = input.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
            if plugin.is_empty() {
                return Err(crate::EngineError::Other("plugins.promote requires a `plugin` id".into()));
            }
            let assertions = input.get("assertions").cloned().unwrap_or(serde_json::json!([]));
            crate::plugins::promote_sidecar(&store, plugin, &assertions)?
        }
        "plugins.lock" => {
            let ap = crate::plugins::active(root, &config);
            let path = scc_plugin_host::write_lockfile(root, &ap.plugins).map_err(crate::EngineError::Other)?;
            serde_json::json!({"ok": true, "path": path.to_string_lossy(), "plugins": crate::plugins::lock_entries(&ap)})
        }
        "plugins.graph" => {
            let ap = crate::plugins::active(root, &config);
            let exts = crate::plugins::collect_extensions(&ap);
            let order = crate::plugins::order_extensions(&exts)?;
            // Group by type in deterministic key order, listing
            // `type:id` in execution order with priority.
            let mut by_type: std::collections::BTreeMap<String, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
            for i in order {
                let e = &exts[i];
                by_type.entry(e.extension_type.clone()).or_default().push(
                    serde_json::json!({"id": e.id, "key": e.key(), "priority": e.priority}),
                );
            }
            serde_json::json!({"groups": by_type})
        }
        "plugins.check" => {
            let ap = crate::plugins::active(root, &config);
            match scc_plugin_host::check_lockfile(root, &ap.plugins) {
                Ok(()) => serde_json::json!({"ok": true}),
                Err(drift) => serde_json::json!({"ok": false, "drift": drift}),
            }
        }
        "plugins.enable" | "plugins.disable" => {
            // Project allow-list (§29 enable/disable): mutate
            // `plugins.enabled` in .scc/config.yaml in place, preserving
            // every other key (raw YAML edit, not a struct round-trip —
            // unknown keys must survive). Empty list means "all
            // discovered run"; enable adds the id, disable removes it.
            // Undiscovered ids fail loudly — a typo must not silently
            // narrow the set.
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                return Err(crate::EngineError::Other(format!("{operation} requires an `id`")));
            }
            let known: Vec<String> = scc_plugin_host::discover(root).into_iter().map(|p| p.manifest.id).collect();
            if !known.iter().any(|k| k == id) {
                return Err(crate::EngineError::Other(format!("unknown plugin '{id}' (discovered: {})", known.join(", "))));
            }
            let path = crate::workspace::config_path(root);
            let mut doc: serde_yaml::Value = if path.exists() {
                let text = std::fs::read_to_string(&path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
                serde_yaml::from_str(&text).map_err(|e| crate::EngineError::Other(e.to_string()))?
            } else {
                serde_yaml::from_str(&scc_indexer::Config::default_yaml()).map_err(|e| crate::EngineError::Other(e.to_string()))?
            };
            if !doc.get("plugins").is_some() {
                if let Some(obj) = doc.as_mapping_mut() {
                    obj.insert(serde_yaml::Value::String("plugins".into()), serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                }
            }
            let mut cur: Vec<String> = doc.get("plugins").and_then(|p| p.get("enabled")).and_then(|v| serde_yaml::from_value(v.clone()).ok()).unwrap_or_default();
            if operation == "plugins.enable" {
                if !cur.iter().any(|x| x == id) { cur.push(id.to_string()); }
            } else {
                cur.retain(|x| x != id);
            }
            if let Some(plugins) = doc.get_mut("plugins").and_then(|p| p.as_mapping_mut()) {
                plugins.insert(serde_yaml::Value::String("enabled".into()), serde_yaml::to_value(&cur).map_err(|e| crate::EngineError::Other(e.to_string()))?);
            }
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| crate::EngineError::Other(e.to_string()))?; }
            let text = serde_yaml::to_string(&doc).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            std::fs::write(&path, text).map_err(|e| crate::EngineError::Other(e.to_string()))?;
            let cfg = crate::workspace::load_config(root)?;
            let ap = crate::plugins::active(root, &cfg);
            serde_json::json!({"ok": true, "enabled": cfg.plugins.enabled, "active": ap.plugins.iter().map(|p| &p.manifest.id).collect::<Vec<_>>()})
        }
        "plugin_state.get" | "plugin_state.put" | "plugin_state.delete" | "plugin_state.scan" => {
            let pid = input.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
            let ap = crate::plugins::active(root, &config);
            let plug = ap.plugins.iter().find(|p| p.manifest.id == pid).ok_or_else(|| {
                crate::EngineError::Other(format!("unknown plugin '{pid}' (not active)"))
            })?;
            match operation {
                "plugin_state.get" => {
                    let key = input.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    crate::state::plugin_state_get(&store, pid, &plug.grants, key)?
                }
                "plugin_state.put" => {
                    let key = input.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    let value = input.get("value").map(|v| v.to_string()).unwrap_or_default();
                    crate::state::plugin_state_put(&store, pid, &plug.grants, key, &value)?
                }
                "plugin_state.delete" => {
                    let key = input.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    crate::state::plugin_state_delete(&store, pid, &plug.grants, key)?
                }
                _ => {
                    let prefix = input.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
                    crate::state::plugin_state_scan(&store, pid, &plug.grants, prefix, limit)?
                }
            }
        }
        "sidecar.put" | "sidecar.get" | "sidecar.scan" => {
            // Raw sidecar storage (§124 item 35): (plugin, graph, key)
            // namespaced analyzer facts. Grant-gated like plugin state;
            // never consumed by ranking/context — promotion only.
            let pid = input.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
            let ap = crate::plugins::active(root, &config);
            let plug = ap.plugins.iter().find(|p| p.manifest.id == pid).ok_or_else(|| {
                crate::EngineError::Other(format!("unknown plugin '{pid}' (not active)"))
            })?;
            match operation {
                "sidecar.put" => {
                    let graph = input.get("graph").and_then(|v| v.as_str()).unwrap_or("default");
                    let key = input.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    let value = input.get("value").map(|v| v.to_string()).unwrap_or_default();
                    crate::state::sidecar_put(&store, pid, &plug.grants, graph, key, &value)?
                }
                "sidecar.get" => {
                    let graph = input.get("graph").and_then(|v| v.as_str()).unwrap_or("default");
                    let key = input.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    crate::state::sidecar_get(&store, pid, &plug.grants, graph, key)?
                }
                _ => {
                    let graph = input.get("graph").and_then(|v| v.as_str()).unwrap_or("default");
                    let prefix = input.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
                    crate::state::sidecar_scan(&store, pid, &plug.grants, graph, prefix, limit)?
                }
            }
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
    // Extension registrations (§17) select hook wiring; legacy bare
    // operation names (ranking.seed/feature/rerank) keep working.
    // trace:exempt reason=internal-detail
    struct PlugSpec { id: String, ops: Vec<String>, exts: Vec<(String, String)> }
    let specs: Vec<PlugSpec> = ap.plugins.iter()
        .map(|p| PlugSpec { id: p.manifest.id.clone(), ops: p.manifest.operations.clone(),
            exts: p.manifest.extensions.iter().map(|e| (e.extension_type.clone(), e.id.clone())).collect() })
        .collect();
    for spec in specs {
        let plug = match ap.plugins.iter().find(|p| p.manifest.id == spec.id).cloned() {
            Some(p) => Arc::new(p),
            None => continue,
        };
        let pid = spec.id.clone();
        let wants = |t: &str, op: &str| {
            spec.exts.iter().any(|(ty, _)| ty == t) || spec.ops.iter().any(|o| o == op)
        };
        if wants("candidate-provider", "ranking.candidates") {
            let plug = Arc::clone(&plug);
            let pid2 = pid.clone();
            hooks.candidates.push(Box::new(move |goal: &str| {
                let input = serde_json::json!({"goal": goal});
                match scc_plugin_host::call(&plug, "ranking.candidates", input, None) {
                    Ok(v) => v.get("candidates").and_then(|s| s.as_array()).map(|a| {
                        a.iter().filter_map(|e| {
                            let id = e.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            if id.is_empty() { return None; }
                            Some(scc_context::rank::ScoredEntity {
                                id,
                                kind: e.get("kind").and_then(|x| x.as_str()).unwrap_or("symbol").into(),
                                name: e.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
                                score: e.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0),
                                reason: format!("plugin:{}", pid2),
                            })
                        }).collect()
                    }).unwrap_or_default(),
                    Err(_) => Vec::new(),
                }
            }));
        }
        if wants("seed-provider", "ranking.seed") {
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
        if wants("rank-feature", "ranking.feature") {
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
        if wants("rank-edge", "ranking.rank_edges") {
            // §48: rank-time edges without canonical facts. One call per
            // request (input: goal); the plugin answers
            // `{"edges": [{subject, predicate, object, weight}]}`. Unknown
            // ids and bad weights degrade inside the ranker, never fail.
            let plug = Arc::clone(&plug);
            let g = goal.to_string();
            hooks.rank_edges.push(Box::new(move |_goal| {
                let input = serde_json::json!({"goal": g});
                match scc_plugin_host::call(&plug, "ranking.rank_edges", input, None) {
                    Ok(v) => v.get("edges").and_then(|s| s.as_array()).map(|a| {
                        a.iter().filter_map(|e| {
                            let (sub, pred, obj) = (
                                e.get("subject").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                e.get("predicate").and_then(|x| x.as_str()).unwrap_or("calls").to_string(),
                                e.get("object").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            );
                            let w = e.get("weight").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            if sub.is_empty() || obj.is_empty() { return None; }
                            Some((sub, pred, obj, w))
                        }).collect()
                    }).unwrap_or_default(),
                    Err(_) => Vec::new(),
                }
            }));
        }
        if wants("criticality-provider", "ranking.criticality") {
            // §53: per-symbol criticality override in [0,1]; None abstains.
            // One call per (symbol, goal) pair at blend time; failures and
            // out-of-range values abstain (resolve_override degrades).
            let plug = Arc::clone(&plug);
            hooks.criticality.push(Box::new(move |sym: &str, goal: &str| {
                let input = serde_json::json!({"symbol": sym, "goal": goal});
                match scc_plugin_host::call(&plug, "ranking.criticality", input, None) {
                    Ok(v) => v.get("criticality").and_then(|x| x.as_f64()),
                    Err(_) => None,
                }
            }));
        }
        if wants("novelty-provider", "ranking.novelty") {
            // §53: per-symbol novelty override in [0,1]; None abstains.
            let plug = Arc::clone(&plug);
            hooks.novelty.push(Box::new(move |sym: &str, goal: &str| {
                let input = serde_json::json!({"symbol": sym, "goal": goal});
                match scc_plugin_host::call(&plug, "ranking.novelty", input, None) {
                    Ok(v) => v.get("novelty").and_then(|x| x.as_f64()),
                    Err(_) => None,
                }
            }));
        }
        if wants("risk-provider", "ranking.risk") {
            // §53: per-symbol change-risk override in [0,1]; None abstains.
            let plug = Arc::clone(&plug);
            hooks.risk.push(Box::new(move |sym: &str, goal: &str| {
                let input = serde_json::json!({"symbol": sym, "goal": goal});
                match scc_plugin_host::call(&plug, "ranking.risk", input, None) {
                    Ok(v) => v.get("risk").and_then(|x| x.as_f64()),
                    Err(_) => None,
                }
            }));
        }
        if wants("semantic-provider", "ranking.semantic") {
            // §53: per-symbol semantic relevance in [0,1]; None abstains.
            let plug = Arc::clone(&plug);
            hooks.semantic.push(Box::new(move |sym: &str, goal: &str| {
                let input = serde_json::json!({"symbol": sym, "goal": goal});
                match scc_plugin_host::call(&plug, "ranking.semantic", input, None) {
                    Ok(v) => v.get("semantic").and_then(|x| x.as_f64()),
                    Err(_) => None,
                }
            }));
        }
        if wants("edge-weight", "ranking.edge_weight") {
            let plug = Arc::clone(&plug);
            hooks.edge_weights.push(std::sync::Arc::new(move |subject, predicate, object, base| {
                let input = serde_json::json!({
                    "subject": subject, "predicate": predicate,
                    "object": object, "base": base,
                });
                match scc_plugin_host::call(&plug, "ranking.edge_weight", input, None) {
                    Ok(v) => {
                        let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let value = v.get("value").and_then(|x| x.as_f64()).unwrap_or(0.0);
                        match mode.as_str() {
                            "add" | "multiply" | "replace" | "veto" => Some((mode, value)),
                            _ => None,
                        }
                    }
                    Err(_) => None,
                }
            }));
        }
        // Named blend profiles (spec 12 + DoD 25): a
        // `blend-profile:<name>` extension registers profile `<name>` via
        // the plugin's `ranking.profile` op returning weight overrides.
        for eid in spec.exts.iter().filter(|(ty, _)| ty == "blend-profile").map(|(_, id)| id) {
            let name = eid.strip_prefix("blend-profile:").unwrap_or(eid).to_string();
            let plug = Arc::clone(&plug);
            if let Ok(v) = scc_plugin_host::call(&plug, "ranking.profile", serde_json::json!({"profile": name}), None) {
                let w = v.get("weights").cloned().unwrap_or(serde_json::Value::Null);
                let mut bw = crate::ranking::BlendWeights::default();
                let mut bad: Vec<String> = Vec::new();
                if let Some(obj) = w.as_object() {
                    for (k, val) in obj {
                        let num = val.as_f64();
                        match (k.as_str(), num) {
                            ("task_ppr", Some(x)) => bw.task_ppr = Some(x),
                            ("global_ppr", Some(x)) => bw.global_ppr = Some(x),
                            ("lexical", Some(x)) => bw.lexical = Some(x),
                            ("semantic", Some(x)) => bw.semantic = Some(x),
                            ("confidence", Some(x)) => bw.confidence = Some(x),
                            ("criticality", Some(x)) => bw.criticality = Some(x),
                            ("change_risk", Some(x)) => bw.change_risk = Some(x),
                            ("novelty", Some(x)) => bw.novelty = Some(x),
                            _ => bad.push(k.clone()),
                        }
                    }
                }
                // Unknown feature keys fail loudly at registration, not
                // silently ignored (would lie about the blend).
                if !bad.is_empty() {
                    ap.diagnostics.push(scc_plugin_host::PluginDiagnostic {
                        plugin: spec.id.clone(), operation: "ranking.profile".into(),
                        error: format!("unknown blend features for profile '{name}': {}", bad.join(", ")),
                        action: "skipped".into(),
                    });
                    continue;
                }
                hooks.profiles.insert(name, bw);
            }
        }
        if wants("coverage", "ranking.coverage") {
            let plug = Arc::clone(&plug);
            hooks.coverage.push(Box::new(move |goal: &str| {
                let input = serde_json::json!({"goal": goal});
                match scc_plugin_host::call(&plug, "ranking.coverage", input, None) {
                    Ok(v) => v.get("required").and_then(|a| a.as_array()).map(|a| {
                        a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()
                    }).unwrap_or_default(),
                    Err(_) => Vec::new(),
                }
            }));
        }
        if wants("similarity", "ranking.similarity") {
            let plug = Arc::clone(&plug);
            hooks.similarities.push(std::sync::Arc::new(move |a, b, ga, gb| {
                let input = serde_json::json!({"a": a, "b": b, "group_a": ga, "group_b": gb});
                match scc_plugin_host::call(&plug, "ranking.similarity", input, None) {
                    Ok(v) => v.get("similarity").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    Err(_) => 0.0,
                }
            }));
        }
        if wants("reranker", "ranking.rerank") {
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
    }
    hooks
}

// trace:exempt reason=internal-detail
fn invoke_context(
    ctx: &crate::SccContext,
    store: &scc_store::Store,
    config: &scc_indexer::Config,
    operation: &str,
    input: Value,
) -> crate::Result<Value> {
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
            let mut pack = ctx.verify(unbounded)?;
            // Plugin verify diagnostics (§124 item 30): appended by the
            // engine so every transport delivers them. Notes inline (the
            // verify pack surfaces warnings as content).
            let (sections, notes) = crate::plugins::verify_diagnostics(&store.root, config);
            pack.content.push_str(&sections);
            for n in notes {
                pack.content.push_str(&format!("\n(verify diagnostic skipped: {n})\n"));
            }
            serde_json::to_value(pack)?
        }
        "context.structural" | "source.structural" => {
            let req: scc_api::StructuralRequest = serde_json::from_value(input)?;
            let goal = req.task.clone().unwrap_or_default();
            let (scorer, _) = crate::inference::rankers(store, config, &goal);
            let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            // §73: units model first — external programs use the
            // structured units without scraping text. Text comes from
            // ctx.structural (owns the HANDLE REFUSED / empty envelopes);
            // units come from structural_units (model data only).
            let text = ctx.structural(&req, &store.root, semantic)?;
            let units = ctx.structural_units(&req, &store.root, semantic)?;
            serde_json::json!({"text": text, "units": units})
        }
        "surface.compile" => {
            serde_json::to_value(ctx.surface_map()?)?
        }
        "surface.render" => {
            let req: scc_api::SurfaceRequest = serde_json::from_value(input)?;
            let goal = req.task.clone().unwrap_or_default();
            let (scorer, _) = crate::inference::rankers(store, config, &goal);
            let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let (result, _) = ctx.surface(&req, semantic)?;
            serde_json::json!({ "text": result.text })
        }
        "surface.build" | "surface.global" | "surface.task" | "ranking.important" | "surface.important" => {
            let req: scc_api::SurfaceRequest = serde_json::from_value(input)?;
            let goal = req.task.clone().unwrap_or_default();
            let (scorer, _) = crate::inference::rankers(store, config, &goal);
            let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let (result, text) = ctx.surface(&req, semantic)?;
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
// trace:v1 id=impl.scc-engine-invoke.exporter-plugins work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
fn plugin_export(
    root: &std::path::Path,
    config: &scc_indexer::Config,
    format: &str,
) -> Option<Value> {
    // `exporter:<format>` extensions render a format the engine does not
    // know (e.g. SARIF, GraphML). Input carries the requested format;
    // output is verbatim `{"format-available": true, "text"|"json"}`.
    let ap = crate::plugins::active(root, config);
    let specs: Vec<(String, String, String)> = ap
        .plugins
        .iter()
        .flat_map(|pl| {
            pl.manifest.extensions.iter()
                .filter(|e| e.extension_type == "exporter" && e.id.strip_prefix("exporter:").unwrap_or(&e.id) == format)
                .map(|e| (pl.manifest.id.clone(), e.id.clone(), pl.manifest.failure_policy.clone()))
        })
        .collect();
    for (pid, ext_id, policy) in specs {
        let plug = match ap.plugins.iter().find(|pl| pl.manifest.id == pid).cloned() {
            Some(pl) => pl,
            None => continue,
        };
        match scc_plugin_host::call(&plug, "export.render", serde_json::json!({"format": format}), None) {
            Ok(v) => {
                let has = v.get("format-available").and_then(|x| x.as_bool()).unwrap_or(false);
                let body = v.get("text").or_else(|| v.get("json"));
                if has && body.is_some() {
                    // Provenance tags the output so consumers know a plugin
                    // rendered it — never a native export masquerading.
                    let mut out = serde_json::json!({"format": format, "plugin": pid, "extension": ext_id});
                    out["output"] = body.cloned().unwrap();
                    return Some(out);
                }
            }
            Err(e) => {
                if policy == "required" {
                    return Some(serde_json::json!({"error": format!("exporter {ext_id} from {pid} FAILED: {e}")}));
                }
            }
        }
    }
    None
}

// trace:exempt reason=internal-detail
fn export_value(store: &scc_store::Store, root: &std::path::Path, format: &str) -> crate::Result<Value> {
    // Plugin exporters (§124 item 31) fire first for unknown formats; the
    // engine falls back to built-ins. `config` loads here (not threaded
    // through every caller) so the seam stays one line at each site.
    if let Ok(config) = crate::workspace::load_config(root) {
        if let Some(v) = plugin_export(root, &config, format) {
            if v.get("error").is_some() {
                let msg = v["error"].as_str().unwrap_or("exporter failed");
                return Err(crate::EngineError::Other(msg.into()));
            }
            return Ok(v);
        }
    }
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
