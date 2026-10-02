//! Plugin precision resolver: a `resolver`-extension process plugin
//! answering `resolution.resolve` upgrades EXTRACTED call edges to
//! RESOLVED through the same validate-then-commit path as LSP backends.

// trace:exempt reason=test-helper
fn write_resolver_plugin(root: &std::path::Path) {
    let plugdir = root.join(".scc").join("plugins").join("acme.resolve");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.resolve\"\nname = \"Resolve\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"resolution.resolve\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"resolver:acme.resolve\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    )
    .unwrap();
    // Upgrade every candidate call to the sibling helper symbol: the target
    // id is derived from the subject's repo prefix (no hardcoded repo id).
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    use std::io::Write;
    f.write_all(
        b"import json, sys\nreq = json.load(sys.stdin)\nups = []\nfor c in req[\"input\"][\"calls\"]:\n    tgt = c[\"subject\"].rsplit(\"/\", 1)[0] + \"/helper\"\n    ups.append({\"subject\": c[\"subject\"], \"object\": c[\"object\"], \"line\": c[\"line\"], \"evidence\": c[\"evidence\"], \"target\": tgt})\nprint(json.dumps({\"output\": {\"upgrades\": ups}}))\n",
    )
    .unwrap();
}

#[test]
// trace:v1 id=test.scc-indexer-resolver.plugin-upgrades-extracted verifies=REQ-SI-503JSBGP exercises=impl.scc-indexer-resolver.plugin-resolvers
fn plugin_resolver_upgrades_extracted_edges() {
    // pyright would race the plugin on this fixture, so disable the
    // builtin path: hand-built model with one EXTRACTED edge, plugin only.
    use scc_core::{Evidence, EvidenceType, Provenance, Relationship};
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    let store = scc_store::Store::open(&root.join(".scc").join("scc.db"), &root).unwrap();
    store.upsert_file("main.py", "hash-test", "python", "source", 100).unwrap();
    let repo_id = store.repository().id;
    let caller = format!("{repo_id}/symbol/main.py/caller");
    let guess = format!("{repo_id}/symbol/main.py/guess");
    let target = format!("{repo_id}/symbol/main.py/helper");
    for id in [&caller, &guess, &target] {
        let short = id.rsplit('/').next().unwrap_or("x").to_string();
        store
            .insert_entity(&scc_core::Entity::new(id, "symbol", short), &["test".to_string()])
            .unwrap();
    }
    let ev = Evidence {
        id: "evidence:test-call".into(),
        r#type: EvidenceType::Source,
        path: Some("main.py".into()),
        symbol: Some("guess".into()),
        start_line: Some(3),
        end_line: None,
        revision: None,
        content_hash: None,
        extractor: Some("test".into()),
        extractor_version: None,
    };
    store.insert_evidence(&ev).unwrap();
    store
        .insert_relationship(
            &Relationship::new("rel-test-1", caller.clone(), scc_core::predicates::CALLS, guess.clone(), Provenance::Extracted)
                .with_evidence(vec![ev.id.clone()]),
            "main.py",
        )
        .unwrap();
    drop(store);
    write_resolver_plugin(&root);
    let store = scc_engine::workspace::open_store(&root).unwrap();
    let files = scc_indexer::resolver::files_with_candidate_edges(&store, 500).unwrap();
    assert!(!files.is_empty(), "candidate file present");
    let report = scc_indexer::resolver::plugin_resolutions(&store, &root, &files).unwrap();
    assert!(report.upgraded > 0, "plugin resolver must upgrade edges: {report:?}");
    assert!(
        report.backends_used.iter().any(|b| b == "plugin:acme.resolve"),
        "plugin backend recorded: {report:?}"
    );
    drop(store);
    let store = scc_engine::workspace::open_store(&root).unwrap();
    let resolved: Vec<_> = store
        .all_relationships()
        .unwrap()
        .into_iter()
        .filter(|r| r.predicate == scc_core::predicates::CALLS && r.provenance == scc_core::Provenance::Resolved)
        .collect();
    assert!(!resolved.is_empty(), "RESOLVED edges present");
    assert!(resolved.iter().all(|r| r.confidence == 0.99), "plugin upgrades carry exact confidence: {resolved:?}");
    assert!(
        resolved.iter().all(|r| r.object == target),
        "upgrade pins the helper target: {resolved:?}"
    );
}

#[test]
// trace:v1 id=test.scc-indexer-resolver.plugin-invents-nothing verifies=REQ-SI-503JSBGP exercises=impl.scc-indexer-resolver.plugin-resolvers
fn plugin_invented_target_fails_file_cleanly() {
    use scc_core::{Evidence, EvidenceType, Provenance, Relationship};
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    // Hand-built model: one EXTRACTED calls edge + its evidence + a real
    // target symbol. No pyright/tsserver involved (unit scope).
    std::fs::create_dir_all(root.join(".scc")).unwrap();
    let store = scc_store::Store::open(&root.join(".scc").join("scc.db"), &root).unwrap();
    store.upsert_file("main.py", "hash-test", "python", "source", 100).unwrap();
    let repo_id = store.repository().id;
    let caller = format!("{repo_id}/symbol/main.py/caller");
    let guess = format!("{repo_id}/symbol/main.py/guess");
    let target = format!("{repo_id}/symbol/main.py/helper");
    for id in [&caller, &guess, &target] {
        let short = id.rsplit('/').next().unwrap_or("x").to_string();
        store
            .insert_entity(
                &scc_core::Entity::new(id, "symbol", short),
                &["test".to_string()],
            )
            .unwrap();
    }
    let ev = Evidence {
        id: "evidence:test-call".into(),
        r#type: EvidenceType::Source,
        path: Some("main.py".into()),
        symbol: Some("guess".into()),
        start_line: Some(3),
        end_line: None,
        revision: None,
        content_hash: None,
        extractor: Some("test".into()),
        extractor_version: None,
    };
    store.insert_evidence(&ev).unwrap();
    store
        .insert_relationship(
            &Relationship::new(
                "rel-test-1",
                caller.clone(),
                scc_core::predicates::CALLS,
                guess.clone(),
                Provenance::Extracted,
            )
            .with_evidence(vec![ev.id.clone()]),
            "main.py",
        )
        .unwrap();
    drop(store);
    // Bad plugin: invents a target naming no entity.
    let plugdir = root.join(".scc").join("plugins").join("acme.bad");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.bad\"\nname = \"Bad\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"resolution.resolve\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"resolver:acme.bad\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    )
    .unwrap();
    let mut f = std::fs::File::create(plugdir.join("plugin.py")).unwrap();
    use std::io::Write;
    f.write_all(
        b"import json, sys\nreq = json.load(sys.stdin)\nups = [dict(subject=c[\"subject\"], object=c[\"object\"], line=c[\"line\"], evidence=c[\"evidence\"], target=\"repo://nowhere/symbol/ghost\") for c in req[\"input\"][\"calls\"]]\nprint(json.dumps({\"output\": {\"upgrades\": ups}}))\n",
    )
    .unwrap();
    let store = scc_engine::workspace::open_store(&root).unwrap();
    let files = scc_indexer::resolver::files_with_candidate_edges(&store, 500).unwrap();
    assert!(!files.is_empty(), "candidate file present");
    let before = store.all_relationships().unwrap();
    let report = scc_indexer::resolver::plugin_resolutions(&store, &root, &files).unwrap();
    assert!(report.errors > 0, "invented target must error: {report:?}");
    assert_eq!(report.upgraded, 0, "nothing upgraded: {report:?}");
    let after = store.all_relationships().unwrap();
    assert_eq!(
        before.iter().map(|r| &r.id).collect::<Vec<_>>(),
        after.iter().map(|r| &r.id).collect::<Vec<_>>(),
        "failed file leaves edges untouched"
    );
}
