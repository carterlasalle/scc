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
