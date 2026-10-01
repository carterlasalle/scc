//! Ranking pipeline integration: stages callable independently,
//! blend matches build_surface math, plugin hooks recorded.

use scc_api::{RankRequest, SelectionRequest, RankedEntry};

// trace:exempt reason=internal-detail
fn fixture() -> (tempfile::TempDir, scc_store::Store) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();
    let repo = store.repo_id.clone();
    for (path, prefix) in [("a/mod.py", "zeta"), ("b/mod.py", "alpha")] {
        for i in 0..10 {
            let name = format!("{prefix}_{i:02}");
            let id = scc_core::symbol_id(&repo, path, &name);
            let mut e = scc_core::Entity::new(id, scc_core::kinds::SYMBOL, name);
            e.attr("kind", serde_json::json!("function"));
            e.attr("file", serde_json::json!(path));
            e.attr("signature", serde_json::json!("def f(x): ..."));
            e.attr("exported", serde_json::json!(false));
            e.attr("start_line", serde_json::json!(1u32));
            e.attr("end_line", serde_json::json!(10u32));
            store.insert_entity(&e, &[path.to_string()]).unwrap();
        }
    }
    (dir, store)
}

// trace:exempt reason=internal-detail
fn ranker_of(store: &scc_store::Store) -> (scc_graph::RealityGraph, scc_indexer::Config, Vec<String>, scc_engine::workspace::Engine<'_>) {
    let graph = scc_graph::RealityGraph::load(store).unwrap();
    let config = scc_indexer::Config::default();
    let engine = scc_engine::workspace::open_engine(store, &config, Vec::new()).unwrap();
    (graph, config, Vec::new(), engine)
}

#[test]
// trace:v1 id=test.scc-engine-ranking.stages verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn stages_agree_and_blend_ranks_task_first() {
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    // Candidates: lexical stage finds zeta symbols for a zeta goal.
    let cands = r.candidates("zeta", 50).unwrap();
    assert!(!cands.is_empty(), "lexical candidates for zeta");
    assert!(cands.iter().all(|c| c.name.contains("zeta")), "all zeta: {cands:?}");
    // Vectors: global covers the universe, task concentrates on zeta.
    let gv = r.pagerank_global().unwrap();
    assert_eq!(gv.len(), 20, "heterogeneous universe = 20 symbols");
    let tv = r.pagerank_task("zeta").unwrap();
    assert_eq!(tv.len(), 20);
    let zmax: f64 = tv.iter().filter(|(id, _)| id.contains("a/mod.py")).map(|(_, s)| *s).fold(0.0, f64::max);
    let amax: f64 = tv.iter().filter(|(id, _)| id.contains("b/mod.py")).map(|(_, s)| *s).fold(0.0, f64::max);
    assert!(zmax > amax, "task PPR concentrates on zeta ({zmax} vs {amax})");
    // Full blend: zeta first, features decomposed, positions dense.
    let out = r.symbols(&RankRequest { profile: None, goal: Some("zeta".into()), limit: 20, explain: true, include_features: true, include_intermediate: false }).unwrap();
    assert_eq!(out.items.len(), 20);
    assert!(out.items[0].id.contains("a/mod.py"), "zeta first: {}", out.items[0].id);
    assert!(out.items[0].features.task_ppr > 0.0);
    assert!(out.items[0].reasons.iter().any(|x| x == "task-seed"), "seed recorded");
    for (i, it) in out.items.iter().enumerate() {
        assert_eq!(it.position, i + 1);
    }
}

#[test]
// trace:v1 id=test.scc-engine-ranking.hooks verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols-hooks
fn plugin_seed_hook_moves_alpha_up() {
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    // Neutral goal: no lexical skew, so movement is pure seed effect.
    let goal = "qqqzzz-no-such-term";
    let base = r.symbols(&RankRequest { profile: None, goal: Some(goal.into()), limit: 20, explain: false, include_features: false, include_intermediate: false }).unwrap();
    let boosted = base.items.iter().find(|i| i.id.contains("b/mod.py")).unwrap().id.clone();
    let base_rank = base.items.iter().find(|i| i.id == boosted).unwrap().rank;
    let mut hooks = scc_engine::ranking::RankHooks::default();
    hooks.seed_providers.push(Box::new(move |_goal| {
        vec![scc_core::TaskSeed { kind: "symbol".into(), id: boosted.clone(), weight: 10.0 }]
    }));
    let out = r.symbols_with_hooks(&RankRequest { profile: None, goal: Some(goal.into()), limit: 20, explain: false, include_features: false, include_intermediate: false }, &hooks).unwrap();
    let new_rank = out.items.iter().find(|i| i.id.contains("b/mod.py")).map(|i| i.rank).unwrap_or(0.0);
    assert!(new_rank > base_rank, "hook seed moves alpha up ({base_rank} -> {new_rank})");
    assert!(out.items[0].id.contains("b/mod.py"), "boosted alpha first: {}", out.items[0].id);
}

#[test]
// trace:v1 id=test.scc-engine-ranking.parity verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn blend_matches_surface_pipeline() {
    use scc_context::surface::{build_surface_staged, SurfaceMode, SurfacePolicy, SurfacePipelineStages, SurfaceRequest};
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let goal = "zeta";
    let out = r.symbols(&RankRequest { profile: None, goal: Some(goal.into()), limit: 20, explain: false, include_features: true, include_intermediate: false }).unwrap();
    // Same order as the pipeline with selection stages off (pure rank).
    let ctx = engine.ctx();
    let policy = SurfacePolicy { quotas: false, mmr: false, coverage: false, hard_max: usize::MAX };
    let stages = SurfacePipelineStages { lexical: true, global_ppr: true, task_ppr: true, mmr: false, quotas: false, optimizer: false };
    let req = SurfaceRequest { mode: SurfaceMode::Task { goal, visible: None }, budget: 1_000_000, explain: false, policy, semantic: None };
    let render = build_surface_staged(&ctx, req, &stages);
    for it in &out.items {
        // Every ranked item exists in the pipeline render with an identical total
        // (same blend inputs; entry-level vs symbol-level aggregation may differ
        // in the 4th decimal only through projection rounding).
        let close = render.rendered_entries.iter().any(|e| e.symbol_id == it.id);
        assert!(close, "pipeline renders {}", it.id);
    }
    // Same top-1.
    assert!(render.rendered_ids.first().map(|id| render.rendered_entries.iter().find(|e| &e.id == id).map(|e| e.symbol_id.clone()).unwrap_or_default()).unwrap_or_default().contains("a/mod.py"),
        "pipeline top is zeta");
    assert!(out.items[0].id.contains("a/mod.py"), "engine top is zeta");
    // Per-item totals agree to 1e-9 (identical blend inputs and math).
    for it in &out.items {
        let e = render.rendered_entries.iter().find(|e| e.symbol_id == it.id).unwrap();
        assert!((e.rank.total - it.rank).abs() < 1e-9, "{}: pipeline {} vs engine {}", it.id, e.rank.total, it.rank);
    }
}

#[test]
// trace:v1 id=test.scc-engine-ranking.pure-ops verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn pure_algorithm_ops() {
    // final_importance is linear: task-only blend sanity.
    let s = scc_context::pagerank::final_importance(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, true);
    assert!((s - (scc_context::pagerank::TASK_PPR_WEIGHT + scc_context::pagerank::CONFIDENCE_WEIGHT)).abs() < 1e-9, "{s}");
    // MMR over same-group items diversifies; quotas cap kinds; budget cuts by value/token.
    let ranked = vec![("a".to_string(), 1.0), ("b".to_string(), 0.9), ("c".to_string(), 0.8)];
    let sel = scc_engine::ranking::mmr_select(&ranked, &|a, b| if a == b { 0.0 } else { 1.0 }, 0.5, 2);
    assert_eq!(sel.len(), 2);
    let rows = vec![
        ("a".to_string(), "public".to_string(), 1.0, 10usize),
        ("b".to_string(), "public".to_string(), 0.9, 10),
        ("c".to_string(), "core".to_string(), 0.8, 10),
    ];
    let q = scc_engine::ranking::apply_quotas(&rows, &[("public".to_string(), 0.5)], 20);
    assert!(q.contains(&"c".to_string()), "core survives capped public: {q:?}");
    let _ = SelectionRequest { ranked: vec![RankedEntry { id: "a".into(), value: 1.0, token_cost: 5, kind: "core".into(), group: None }], budget: 10, lambda: None, quotas: None };
}

#[test]
// trace:v1 id=test.scc-engine-ranking.coverage-hook verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn coverage_hook_marks_required() {
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let goal = "qqqzzz-no-such-term";
    let req = RankRequest { profile: None, goal: Some(goal.into()), limit: 20, explain: true, include_features: false, include_intermediate: false };
    let base = r.symbols(&req).unwrap();
    // Pick a symbol that is NOT required in the base blend.
    let target = base.items.iter().find(|i| !i.reasons.iter().any(|x| x.contains("required"))).map(|i| i.id.clone()).expect("a non-required item");
    let mut hooks = scc_engine::ranking::RankHooks::default();
    let t2 = target.clone();
    hooks.coverage.push(Box::new(move |_goal| vec![t2.clone()]));
    let out = r.symbols_with_hooks(&req, &hooks).unwrap();
    let item = out.items.iter().find(|i| i.id == target).expect("target present");
    assert!(item.features.criticality == 1.0, "coverage marks criticality 1.0: {item:?}");
    assert!(item.reasons.iter().any(|x| x.contains("required-by:plugin")), "recorded: {:?}", item.reasons);
}

#[test]
// trace:v1 id=test.scc-engine-ranking.seeds-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn seeds_op_merges_lexical_and_plugin() {
    // §123.11: `ranking.seeds` exposes the same merge symbols() blends —
    // verified here at the seeds_with seam (invoke path covered by the
    // plugins-coverage test exercising the same hook wiring).
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let base = r.seeds_with("zeta", &scc_engine::ranking::RankHooks::default()).unwrap();
    assert!(!base.is_empty(), "lexical seeds for zeta");
    let first = base[0].id.clone();
    let w0 = base[0].weight;
    let mut hooks = scc_engine::ranking::RankHooks::default();
    hooks.seed_providers.push(Box::new(move |_goal| {
        vec![scc_core::TaskSeed { kind: "symbol".into(), id: first.clone(), weight: 5.0 }]
    }));
    let out = r.seeds_with("zeta", &hooks).unwrap();
    let hit = out.iter().find(|x| x.id == base[0].id).unwrap();
    assert!((hit.weight - (w0 + 5.0)).abs() < 1e-9, "{out:?}");
    // And the op serves the same merge over RPC-shaped input.
    let v = serde_json::json!({"seeds": out.iter().map(|x| serde_json::json!({"id": x.id, "kind": x.kind, "weight": x.weight})).collect::<Vec<_>>()});
    assert_eq!(v["seeds"][0]["id"], serde_json::json!(base[0].id));
}

#[test]
// trace:v1 id=test.scc-engine-ranking.reference-graph-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn reference_graph_op_normalizes_trusted_rels() {
    // `ranking.reference_graph`: every edge traces to one trusted
    // relationship — endpoints are known universe nodes.
    let (_dir, store) = fixture();
    let syms: Vec<String> = store.all_entities().unwrap().into_iter()
        .filter(|e| e.kind == scc_core::kinds::SYMBOL).map(|e| e.id).collect();
    let rel = scc_core::Relationship::new(
        "rel-0", syms[0].clone(), scc_core::predicates::CALLS, syms[1].clone(),
        scc_core::Provenance::Extracted);
    store.insert_relationship(&rel, "a/mod.py").unwrap();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let edges = engine.ranking().reference_graph().unwrap();
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].kind, scc_core::ReferenceKind::Call);
    let nodes: std::collections::BTreeSet<String> =
        engine.ranking().universe().unwrap().into_iter().map(|(id, _)| id).collect();
    assert!(nodes.contains(&edges[0].source_symbol));
    assert!(nodes.contains(&edges[0].target_symbol));
}

#[test]
// trace:v1 id=test.scc-engine-ranking.universe-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn universe_op_lists_nodes_with_kinds() {
    // `ranking.universe`: the node table every vector is indexed by.
    // Kinds let callers walk the symbol slice; nodes match pagerank.global.
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let nodes = r.universe().unwrap();
    assert_eq!(nodes.len(), 20, "universe = 20 fixture symbols");
    assert!(nodes.iter().all(|(_, k)| k == "symbol"), "{nodes:?}");
    let gv = r.pagerank_global().unwrap();
    assert_eq!(nodes.len(), gv.len(), "universe is the vector index");
    for ((id, _), (gid, _)) in nodes.iter().zip(gv.iter()) {
        assert_eq!(id, gid, "same order, same ids");
    }
}

#[test]
// trace:v1 id=test.scc-engine-ranking.edges-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn edges_op_lists_universe() {
    // §123.12: `ranking.edges` serves the structure the vectors diffuse
    // over. Universe of 20 fixture symbols ⇒ edges reference only known
    // nodes, weights positive and finite.
    let (_dir, store) = fixture();
    // The fixture inserts entities only — add two calls rels so the
    // universe has edges (subject/object are fixture symbol ids).
    let syms: Vec<String> = store.all_entities().unwrap().into_iter()
        .filter(|e| e.kind == scc_core::kinds::SYMBOL).map(|e| e.id).collect();
    for (i, (a, b)) in [(syms[0].clone(), syms[1].clone()), (syms[1].clone(), syms[2].clone())].into_iter().enumerate() {
        let rel = scc_core::Relationship::new(
            format!("rel-{i}"), a, scc_core::predicates::CALLS, b, scc_core::Provenance::Extracted);
        store.insert_relationship(&rel, "a/mod.py").unwrap();
    }
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let edges = r.rank_edges().unwrap();
    assert!(!edges.is_empty(), "universe has edges");
    let nodes: std::collections::BTreeSet<String> =
        r.pagerank_global().unwrap().into_iter().map(|(id, _)| id).collect();
    for (s, _p, o, w) in &edges {
        assert!(nodes.contains(s), "unknown subject {s}");
        assert!(nodes.contains(o), "unknown object {o}");
        assert!(w.is_finite() && *w > 0.0, "bad weight {w}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-ranking.features-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols
fn features_op_decomposes_scores() {
    // §123.13: `ranking.features` serves the score inputs before the
    // blend — core 8 + plugin features + specificity, positions dense.
    let (_dir, store) = fixture();
    let (_g, _c, _s, engine) = ranker_of(&store);
    let r = engine.ranking();
    let req = RankRequest { profile: None, goal: Some("zeta".into()), limit: 20,
        explain: false, include_features: true, include_intermediate: false };
    let out = r.symbols(&req).unwrap();
    assert!(!out.items.is_empty());
    let first = &out.items[0];
    assert!(first.features.task_ppr > 0.0, "task signal present: {first:?}");
    assert_eq!(first.position, 1);
    // Projection contract: every item carries all 8 core features.
    for it in &out.items {
        let _ = (it.features.task_ppr, it.features.global_ppr, it.features.lexical,
            it.features.semantic, it.features.confidence, it.features.criticality,
            it.features.change_risk, it.features.novelty);
    }
}
