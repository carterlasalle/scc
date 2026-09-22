//! Registry coverage for newly registered portable operations.
//!
//! Drives every new arm through the public `invoke()` entrypoint on a tiny
//! indexed repo — envelopes and failure modes, not ranking quality.

use serde_json::json;

// trace:v1 id=test.scc-engine-operations.fixture work=WORK-SI-MMMJA4G6 verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.model-get
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def hello():\n    return 1\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    (dir, root)
}

#[test]
// trace:v1 id=test.scc-engine-operations.model-get verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.model-get
fn model_get_returns_complete_envelope() {
    let (_dir, root) = fixture();
    let v = scc_engine::invoke(&root, "model.get", json!({})).unwrap();
    for key in [
        "repository", "revision", "epoch", "stats", "files", "entities",
        "relationships", "evidence", "components", "flows", "flow_graphs",
        "invariants",
    ] {
        assert!(v.get(key).is_some(), "model.get missing {key}: {v}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.session-pin verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-workspace.session
fn workspace_session_pins_and_checks() {
    let (_dir, root) = fixture();
    let sess = scc_engine::invoke(&root, "workspace.session", json!({})).unwrap();
    assert!(sess.get("repo_id").is_some(), "session pins repo: {sess}");
    assert!(sess.get("epoch").is_some(), "session pins epoch: {sess}");
    assert!(sess.get("rank_salt").is_some(), "session pins salt: {sess}");
    let check =
        scc_engine::invoke(&root, "workspace.session_check", json!({"session": sess})).unwrap();
    assert_eq!(check.get("current"), Some(&json!(true)), "live session current: {check}");
    let mut stale = sess.clone();
    stale["epoch"] = json!("epoch:tampered");
    let check =
        scc_engine::invoke(&root, "workspace.session_check", json!({"session": stale})).unwrap();
    assert_eq!(check.get("current"), Some(&json!(false)), "tampered session stale: {check}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.evidence-search verifies=REQ-SI-503JSBGP
fn evidence_search_and_signatures() {
    let (_dir, root) = fixture();
    let list = scc_engine::invoke(&root, "evidence.list", json!({})).unwrap();
    assert!(list.is_array(), "evidence.list is an array: {list}");
    let one = scc_engine::invoke(&root, "evidence.get", json!({"id": "no-such-id"})).unwrap();
    assert!(one.is_null(), "unknown evidence id is null: {one}");
    let syms =
        scc_engine::invoke(&root, "graph.search_symbols", json!({"query": "hello"})).unwrap();
    let arr = syms.get("symbols").and_then(|s| s.as_array()).unwrap();
    assert!(!arr.is_empty(), "symbol search finds hello: {syms}");
    let sigs = scc_engine::invoke(&root, "runtime.signatures", json!({})).unwrap();
    assert!(sigs.get("signatures").and_then(|s| s.as_array()).is_some(), "{sigs}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.profile-gate verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ranking.symbols-hooks
fn unknown_ranking_profile_fails_closed() {
    let (_dir, root) = fixture();
    let r = scc_engine::invoke(
        &root,
        "ranking.symbols",
        json!({"goal": "hello", "limit": 5, "profile": "no-such-profile"}),
    );
    assert!(r.is_err(), "unknown profile must fail, not silently default");
    let ok = scc_engine::invoke(
        &root,
        "ranking.symbols",
        json!({"goal": "hello", "limit": 5, "profile": "default"}),
    )
    .unwrap();
    assert!(ok.get("items").and_then(|i| i.as_array()).is_some(), "{ok}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.export-aliases verifies=REQ-SI-503JSBGP
fn export_aliases_resolve() {
    let (_dir, root) = fixture();
    let jsonl = scc_engine::invoke(&root, "export.system_ir_jsonl", json!({})).unwrap();
    assert!(jsonl.as_array().is_some_and(|a| !a.is_empty()), "{jsonl}");
    let ccg = scc_engine::invoke(&root, "export.ccg", json!({})).unwrap();
    assert_eq!(ccg.get("schema"), Some(&json!("ccg")), "{ccg}");
    let snap = scc_engine::invoke(&root, "export.snap", json!({})).unwrap();
    assert!(snap.as_str().is_some_and(|s| s.contains("SYSTEM CAPSULE")), "{snap}");
    let unknown = scc_engine::invoke(&root, "integrations.describe", json!({"name": "nope"}));
    let _ = unknown;
}

#[test]
// trace:v1 id=test.scc-engine-operations.subagent-compress verifies=REQ-SI-503JSBGP
fn subagent_and_compress_derive() {
    let (_dir, root) = fixture();
    let sub = scc_engine::invoke(&root, "context.subagent", json!({"goal": "hello"})).unwrap();
    assert!(
        sub.get("content").and_then(|c| c.as_str()).is_some_and(|c| c.contains("SUBAGENT SCOPE")),
        "{sub}"
    );
    let pack = scc_engine::invoke(&root, "context.compress", json!({"goal": "hello"})).unwrap();
    assert!(pack.get("content").and_then(|c| c.as_str()).is_some(), "{pack}");
}
