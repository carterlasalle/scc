//! Owned-handle facade: `SccEngine::open` for in-process embedding.
//!
//! The typed namespaces and `invoke()` share one implementation (module
//! fns both call); this test proves the facade reaches the same behavior
//! as the transport entrypoint on the same repo.

use serde_json::json;

// trace:exempt reason=test-helper
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def hello():\n    return 1\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    (dir, root)
}

#[test]
// trace:v1 id=test.scc-engine-facade.open-invoke-session verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-facade.handle
fn open_invoke_and_session_round_trip() {
    let (_dir, root) = fixture();
    let engine = scc_engine::facade::SccEngine::open(&root).unwrap();
    assert_eq!(engine.root(), root.as_path());
    // invoke() reaches the registry.
    let status = engine.invoke("workspace.status", json!({})).unwrap();
    assert!(status.get("repository").is_some(), "status via facade: {status}");
    let syms = engine
        .invoke("graph.search_symbols", json!({"query": "hello", "limit": 5}))
        .unwrap();
    assert!(
        syms.get("symbols").and_then(|s| s.as_array()).is_some_and(|a| !a.is_empty()),
        "symbol search via facade: {syms}"
    );
    // Session pins and stays current; invoke_session answers under it.
    let session = engine.session().unwrap();
    assert!(!session.repo_id.is_empty(), "session pins repo: {session:?}");
    let out = engine
        .invoke_session(&session, "workspace.status", json!({}))
        .unwrap();
    assert!(out.get("repository").is_some(), "session invoke: {out}");
    // Typed helpers match invoke() on the same model.
    let via_helper = engine.graph_relationships(None, None, 100).unwrap();
    let via_invoke = engine.invoke("graph.relationships", json!({})).unwrap();
    assert_eq!(
        via_helper.len(),
        via_invoke.as_array().map(|a| a.len()).unwrap_or(usize::MAX),
        "helper and invoke agree"
    );
    let model = engine.model().unwrap();
    assert!(model.get("entities").is_some(), "model via facade: {model}");
}

#[test]
// trace:v1 id=test.scc-engine-facade.namespaces verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-facade.handle
fn spec_usage_namespaces_round_trip() {
    let (_dir, root) = fixture();
    let engine = scc_engine::facade::SccEngine::open(&root).unwrap();
    let atlas = engine.context_atlas(8_000).unwrap();
    assert!(!atlas.content.is_empty(), "atlas via facade");
    let (result, text) = engine.surface_build("hello world", 8_000, true).unwrap();
    assert!(!text.is_empty() && !result.rendered_ids.is_empty(), "surface via facade");
    let ranked = engine.ranking_symbols("hello world", 10, true).unwrap();
    assert!(!ranked.items.is_empty(), "ranking via facade");
    assert!(ranked.items.iter().all(|i| !i.reasons.is_empty()), "explain recorded: {ranked:?}");
    let artifact = engine.context_task("hello world", 8_000).unwrap();
    assert!(!artifact.pack.content.is_empty(), "task pack via facade");
    assert!(artifact.token_count > 0, "token count via facade");
}

#[test]
// trace:v1 id=test.scc-engine-facade.namespace-chain verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-facade.context-ns
fn namespace_chain_matches_flat_methods() {
    let (_dir, root) = fixture();
    let engine = scc_engine::facade::SccEngine::open(&root).unwrap();
    // context_ns / surface_ns / ranking_ns / graph_ns reach the same
    // derivations as the flat facade methods.
    let via_ns = engine.context_ns().atlas(8_000).unwrap();
    let via_flat = engine.context_atlas(8_000).unwrap();
    assert_eq!(via_ns.content, via_flat.content, "atlas chain == flat");
    let (r1, _) = engine.surface_ns().build("hello world", 8_000, false).unwrap();
    let (r2, _) = engine.surface_build("hello world", 8_000, false).unwrap();
    assert_eq!(r1.rendered_ids, r2.rendered_ids, "surface chain == flat");
    let n1 = engine.ranking_ns().symbols("hello world", 10, false).unwrap();
    let n2 = engine.ranking_symbols("hello world", 10, false).unwrap();
    assert_eq!(n1.items.len(), n2.items.len(), "ranking chain == flat");
    let g1 = engine.graph_ns().relationships(None, None, 100).unwrap();
    let g2 = engine.graph_relationships(None, None, 100).unwrap();
    assert_eq!(g1.len(), g2.len(), "graph chain == flat");
    let hit = engine.graph_ns().query("hello", 5).unwrap();
    assert!(!hit.entities.is_empty(), "graph query via chain");
}
