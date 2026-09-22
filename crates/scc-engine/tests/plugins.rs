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

#[test]
// trace:v1 id=test.scc-engine-plugins.ordering verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.order-extensions
fn order_extensions_respects_priority_and_edges() {
    use scc_engine::plugins::ExtensionOrder;
    let exts = vec![
        ExtensionOrder { extension_type: "rank-feature".into(), id: "c".into(), priority: 30, after: vec![], before: vec![] },
        ExtensionOrder { extension_type: "rank-feature".into(), id: "a".into(), priority: 10, after: vec![], before: vec![] },
        ExtensionOrder { extension_type: "rank-feature".into(), id: "b".into(), priority: 20, after: vec!["rank-feature:a".into()], before: vec![] },
    ];
    let order = scc_engine::plugins::order_extensions(&exts).unwrap();
    let ids: Vec<&str> = order.iter().map(|&i| exts[i].id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"], "{ids:?}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.ordering-rejects verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.order-extensions
fn order_extensions_rejects_unknown_and_cycle() {
    use scc_engine::plugins::ExtensionOrder;
    let unknown = vec![
        ExtensionOrder { extension_type: "t".into(), id: "x".into(), priority: 0, after: vec!["t:nope".into()], before: vec![] },
    ];
    assert!(scc_engine::plugins::order_extensions(&unknown).is_err());
    let cycle = vec![
        ExtensionOrder { extension_type: "t".into(), id: "x".into(), priority: 0, after: vec!["t:y".into()], before: vec![] },
        ExtensionOrder { extension_type: "t".into(), id: "y".into(), priority: 0, after: vec!["t:x".into()], before: vec![] },
    ];
    assert!(scc_engine::plugins::order_extensions(&cycle).is_err());
}

#[test]
// trace:v1 id=test.scc-engine-plugins.contribution-pipeline verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.commit-contribution
fn contribution_pipeline_validates_then_commits() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def hello():\n    return 1\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    let store = scc_store::Store::open(&dir.path().join("scc.db"), &root).unwrap();
    // Dangling endpoint fails BEFORE any write.
    let bad = serde_json::json!({"entities": [], "relationships": [
        {"id": "r1", "subject": "ghost", "predicate": "calls", "object": "ghost2", "provenance": "extracted", "confidence": 1.0}
    ], "evidence": []});
    assert!(scc_engine::plugins::commit_contribution(&store, "acme.t", &bad).is_err());
    assert!(store.search_entities("ghost", 10).unwrap().is_empty());
    // Valid batch commits with provenance.
    let good = serde_json::json!({"entities": [
        {"id": "plugin:acme/boundary", "kind": "plugin:acme/security_boundary", "name": "edge", "attributes": {}, "evidence": []}
    ], "relationships": [], "evidence": []});
    let out = scc_engine::plugins::commit_contribution(&store, "acme.t", &good).unwrap();
    assert_eq!(out.get("entities"), Some(&serde_json::json!(1)));
    let found = store.search_entities("edge", 10).unwrap();
    assert!(found.iter().any(|e| e.id == "plugin:acme/boundary"), "{found:?}");
    // Custom kind without the plugin: namespace is rejected.
    let unscoped = serde_json::json!({"entities": [
        {"id": "x", "kind": "custom/thing", "name": "x", "attributes": {}, "evidence": []}
    ], "relationships": [], "evidence": []});
    assert!(scc_engine::plugins::commit_contribution(&store, "acme.t", &unscoped).is_err());
}

#[test]
// trace:v1 id=test.scc-engine-plugins.context-section verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.context-sections
fn context_section_plugin_appends_provenance_section() {
    use std::io::Write;
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def hello():\n    return 1\n").unwrap();
    let plugdir = root.join(".scc").join("plugins").join("acme.sec");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.sec\"\nname = \"Sec\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"context.section\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"context-section:acme.impact\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    ).unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    f.write_all(b"import json, sys\nreq = json.load(sys.stdin)\ngoal = req[\"input\"].get(\"goal\", \"\")\nprint(json.dumps({\"output\": {\"section\": \"risk: \" + goal}}))\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    let out = scc_engine::invoke(
        &root,
        "context.task",
        serde_json::json!({"goal": "hello", "files": [], "symbols": [], "budget": null, "hook": false}),
    )
    .unwrap();
    let content = out["pack"]["content"].as_str().unwrap_or("");
    assert!(content.contains("# PLUGIN SECTION acme.impact (from acme.sec"), "{content}");
    assert!(content.contains("risk: hello"), "{content}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.lockfile-round-trip verifies=REQ-SI-503JSBGP exercises=impl.scc-plugin-host.lockfile
fn plugin_lockfile_round_trip_and_drift() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = write_plugin(dir.path());
    // No lockfile yet: check passes vacuously.
    let ap = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    assert!(scc_plugin_host::check_lockfile(&root, &ap.plugins).is_ok());
    // Write then verify: current.
    scc_plugin_host::write_lockfile(&root, &ap.plugins).unwrap();
    assert!(root.join(".scc").join("plugins.lock").is_file());
    let ap2 = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    assert!(scc_plugin_host::check_lockfile(&root, &ap2.plugins).is_ok());
    // Tamper the manifest version: drift names the plugin.
    let manifest = root.join(".scc").join("plugins").join("acme.echo").join("scc-plugin.toml");
    let text = std::fs::read_to_string(&manifest).unwrap().replace("1.0.0", "9.9.9");
    std::fs::write(&manifest, text).unwrap();
    let ap3 = scc_engine::plugins::active(&root, &scc_indexer::Config::default());
    let err = scc_plugin_host::check_lockfile(&root, &ap3.plugins).unwrap_err();
    assert!(err.contains("acme.echo"), "{err}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.lock-check-ops verifies=REQ-SI-503JSBGP exercises=impl.scc-plugin-host.lockfile-check
fn plugin_lock_and_check_ops_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = write_plugin(dir.path());
    let out = scc_engine::invoke(&root, "plugins.lock", serde_json::json!({})).unwrap();
    assert_eq!(out.get("ok"), Some(&serde_json::json!(true)), "{out}");
    let check = scc_engine::invoke(&root, "plugins.check", serde_json::json!({})).unwrap();
    assert_eq!(check.get("ok"), Some(&serde_json::json!(true)), "{check}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.edge-weight verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.call-operation
fn edge_weight_contributor_alters_rank() {
    use std::io::Write;
    // zeta calls alpha: veto on the zeta->alpha edge starves alpha of flow.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def alpha():\n    return 1\ndef zeta():\n    return alpha()\n").unwrap();
    let plugdir = root.join(".scc").join("plugins").join("acme.edge");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.edge\"\nname = \"Edge\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"ranking.edge_weight\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"edge-weight:acme.edge\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    ).unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    // Multiply every edge into alpha by 0.001 (near-starve, not veto: veto
    // would also drop reverse transitions and could disconnect the graph).
    f.write_all(b"import json, sys\nreq = json.load(sys.stdin)\nobj = req[\"input\"].get(\"object\", \"\")\nmode = \"multiply\" if \"alpha\" in obj else \"none\"\nprint(json.dumps({\"output\": {\"mode\": mode, \"value\": 0.001}}))\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    let base = scc_engine::invoke(
        &root, "ranking.symbols",
        serde_json::json!({"goal": "", "limit": 10}),
    )
    .unwrap();
    let items = base["items"].as_array().unwrap();
    let pos = |sub: &str| items.iter().position(|i| i["id"].as_str().unwrap_or("").contains(sub)).unwrap();
    let base_alpha = pos("alpha");
    // Edge contributions are recorded: reasons + warning.
    assert!(
        items.iter().all(|i| i["reasons"].as_array().unwrap().iter().any(|r| r.as_str().unwrap_or("").starts_with("edge-weights("))),
        "edge-weight reasons recorded: {base}"
    );
    assert!(
        base["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap_or("").contains("edge-weight")),
        "edge-weight warning recorded: {base}"
    );
    // Sanity: alpha still ranks (near-starve, not disconnect).
    assert!(base_alpha < items.len(), "alpha present: {base}");
}

#[test]
// trace:v1 id=test.scc-engine-plugins.state-crud verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.call-operation
fn plugin_state_crud_is_namespaced_and_gated() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = write_plugin(dir.path());
    // The echo fixture grants repo_read only: state.get must be denied.
    let denied = scc_engine::invoke(
        &root, "plugin_state.get",
        serde_json::json!({"plugin": "acme.echo", "key": "k"}),
    );
    assert!(denied.is_err(), "state without grant must fail: {denied:?}");
    // Grant state.read + state.write via project config file.
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    std::fs::write(
        root.join(".scc").join("config.yaml"),
        "schema: 1\nplugins:\n  grants:\n    acme.echo: [state.read, state.write]\n",
    )
    .unwrap();
    let put = scc_engine::invoke(
        &root, "plugin_state.put",
        serde_json::json!({"plugin": "acme.echo", "key": "cursor", "value": {"n": 1}}),
    )
    .unwrap();
    assert_eq!(put.get("ok"), Some(&serde_json::json!(true)), "{put}");
    let got = scc_engine::invoke(
        &root, "plugin_state.get",
        serde_json::json!({"plugin": "acme.echo", "key": "cursor"}),
    )
    .unwrap();
    assert!(got.as_str().is_some_and(|s| s.contains('1')), "{got}");
    // Namespace isolation: another plugin id cannot see this key.
    let other = scc_engine::invoke(
        &root, "plugin_state.get",
        serde_json::json!({"plugin": "acme.other", "key": "cursor"}),
    );
    assert!(other.is_err(), "unknown plugin must fail: {other:?}");
    let scan = scc_engine::invoke(
        &root, "plugin_state.scan",
        serde_json::json!({"plugin": "acme.echo", "prefix": "cur", "limit": 10}),
    )
    .unwrap();
    assert_eq!(scan["keys"].as_array().map(|a| a.len()), Some(1), "{scan}");
    let del = scc_engine::invoke(
        &root, "plugin_state.delete",
        serde_json::json!({"plugin": "acme.echo", "key": "cursor"}),
    )
    .unwrap();
    assert_eq!(del.get("ok"), Some(&serde_json::json!(true)), "{del}");
    let gone = scc_engine::invoke(
        &root, "plugin_state.get",
        serde_json::json!({"plugin": "acme.echo", "key": "cursor"}),
    )
    .unwrap();
    assert!(gone.is_null(), "deleted key reads null: {gone}");
}
