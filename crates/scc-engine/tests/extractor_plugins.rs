//! Plugin language extractor: an `extractor`-extension process plugin
//! answering `extraction.extract` contributes additive `ExtractedFile`
//! rows merged over builtin extraction (merge-only, never replace).

use std::io::Write;

// trace:exempt reason=test-helper
fn write_extractor_plugin(root: &std::path::Path, body: &str) {
    let plugdir = root.join(".scc").join("plugins").join("acme.ext");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.ext\"\nname = \"Ext\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"extraction.extract\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"extractor:acme.ext\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    )
    .unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

#[test]
// trace:v1 id=test.scc-indexer-extractor.plugin-symbols-merge verifies=REQ-SI-503JSBGP exercises=impl.scc-indexer-extractor.plugin-merge
fn plugin_extractor_symbols_merge_over_builtin() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def alpha():\n    return 1\n").unwrap();
    // Plugin contributes one extra symbol row for every file.
    write_extractor_plugin(
        &root,
        "import json, sys\nreq = json.load(sys.stdin)\npath = req[\"input\"][\"path\"]\nprint(json.dumps({\"output\": {\"symbols\": [{\"name\": \"plugin_fn\", \"kind\": \"function\", \"start_line\": 1, \"end_line\": 1, \"exported\": True}]}}))\n",
    );
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    let engine_store = || scc_engine::workspace::open_store(&root).unwrap();
    scc_indexer::Indexer::new(engine_store(), scc_indexer::Config::default())
        .index()
        .unwrap();

    let names: Vec<String> = engine_store()
        .all_entities()
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "symbol")
        .map(|e| e.name)
        .collect();
    assert!(names.iter().any(|n| n == "alpha"), "builtin symbol kept: {names:?}");
    assert!(names.iter().any(|n| n == "plugin_fn"), "plugin symbol merged: {names:?}");
}

#[test]
// trace:v1 id=test.scc-indexer-extractor.plugin-failure-degrades verifies=REQ-SI-503JSBGP exercises=impl.scc-indexer-extractor.plugin-merge
fn plugin_extractor_failure_keeps_builtin_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def alpha():\n    return 1\n").unwrap();
    // Plugin crashes: the file must still index builtin-only.
    write_extractor_plugin(&root, "import sys\nsys.exit(3)\n");
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    let engine_store = || scc_engine::workspace::open_store(&root).unwrap();
    scc_indexer::Indexer::new(engine_store(), scc_indexer::Config::default())
        .index()
        .unwrap();
    let store = engine_store();
    let names: Vec<String> = store
        .all_entities()
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "symbol")
        .map(|e| e.name)
        .collect();
    assert!(names.iter().any(|n| n == "alpha"), "builtin survives plugin crash: {names:?}");
}
