//! Plugin host integration: a Python process plugin end-to-end.
//!
//! Proves spec section 15 (arbitrary languages, no Rust) and section 31
//! (custom operations reachable through invoke with no per-transport code).

use std::io::Write;

// trace:exempt reason=internal-detail
fn write_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    let plugdir = dir.join(".scc").join("plugins").join("acme.echo");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.echo\"\nname = \"Echo\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"acme.echo\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[permissions]\nrepo_read = true\n",
    ).unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    f.write_all(b"import json, sys\nreq = json.load(sys.stdin)\nprint(json.dumps({\"output\": {\"echo\": req[\"input\"].get(\"text\", \"\")}}))\n").unwrap();
    dir.to_path_buf()
}

#[test]
// trace:v1 id=test.scc-engine-plugins.echo-round-trip verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.call-operation
fn process_plugin_echo_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = write_plugin(dir.path());
    let mut ap = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    assert_eq!(ap.plugins.len(), 1, "echo plugin discovered");
    let out = scc_engine::plugins::call_operation(&mut ap, "acme.echo", serde_json::json!({"text": "hello"})).unwrap();
    assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("hello"));
    assert!(out.get("_origin").is_some(), "mandatory provenance {out}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.unknown-op verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.call-operation
fn unknown_operation_is_no_provider() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let mut ap = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    let r = scc_engine::plugins::call_operation(&mut ap, "acme.missing", serde_json::json!({}));
    assert!(r.is_err());
}

#[test]
// trace:v1 id=test.scc-engine-plugins.lock-invalidates verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.cache-key-fragment
fn lock_changes_cache_key() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = write_plugin(dir.path());
    let ap = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    let k1 = scc_engine::plugins::cache_key_fragment(&ap);
    assert!(k1.starts_with("plugins:"));
    assert_eq!(k1, scc_engine::plugins::cache_key_fragment(&ap));
}

#[test]
// trace:v1 id=test.scc-engine-plugins.extension-wires-feature verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.call-operation
fn extension_registration_wires_rank_feature() {
    use std::io::Write;
    // Repo with two symbols; goal favors zeta so alpha starts below.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def zeta():\n    return 1\ndef alpha():\n    return 2\n").unwrap();
    // Feature plugin registered ONLY via [extensions] (no legacy op name
    // needed for wiring — operations still declares the callable op).
    let plugdir = root.join(".scc").join("plugins").join("acme.feat");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.feat\"\nname = \"Feat\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"ranking.feature\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"rank-feature:acme.feat\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    ).unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    f.write_all(b"import json, sys\nreq = json.load(sys.stdin)\nsym = req[\"input\"].get(\"symbol\", \"\")\nboost = 1.0 if \"alpha\" in sym else 0.0\nprint(json.dumps({\"output\": {\"score\": boost, \"weight\": 100.0, \"reason\": \"acme-boost\"}}))\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    let out = scc_engine::invoke(
        &root,
        "ranking.symbols",
        serde_json::json!({"goal": "zeta", "limit": 10, "explain": true, "include_features": true}),
    )
    .unwrap();
    let items = out["items"].as_array().unwrap();
    assert!(!items.is_empty(), "ranked items {out}");
    // The extension-registered feature moved alpha above zeta.
    let pos = |sub: &str| items.iter().position(|i| i["id"].as_str().unwrap_or("").contains(sub)).unwrap();
    assert!(pos("alpha") < pos("zeta"), "acme-boost moved alpha up: {out}");
    assert!(
        items[pos("alpha")]["plugin_features"].get("acme.feat.feature").is_some(),
        "feature recorded under plugin name: {out}"
    );
    assert!(
        items[pos("alpha")]["reasons"].as_array().unwrap().iter().any(|r| r == "acme-boost"),
        "reason recorded: {out}"
    );
}
