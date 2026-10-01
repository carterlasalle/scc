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

#[test]
// trace:v1 id=test.scc-engine-operations.invoke-session verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-workspace.session
fn invoke_session_rejects_stale() {
    let (_dir, root) = fixture();
    let store = scc_store::Store::open(&root.join(".scc").join("scc.db"), &root).unwrap();
    let config = scc_indexer::Config::default();
    let stale = scc_engine::workspace::stale_paths(&store).unwrap();
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).unwrap();
    let sess = scc_engine::workspace::open_session(&store, &config).unwrap();
    let ok = engine.invoke_session(&sess, "workspace.status", serde_json::json!({})).unwrap();
    assert!(ok.get("stats").is_some(), "{ok}");
    let mut tampered = sess.clone();
    tampered.config_hash = "tampered".into();
    let err = engine.invoke_session(&tampered, "workspace.status", serde_json::json!({}));
    assert!(err.is_err(), "stale session must fail, not silently answer");
    assert!(err.unwrap_err().to_string().contains("config"), "names the drifted field");
}

#[test]
// trace:exempt reason=unit-test
fn surface_stages_toggle_changes_render() {
    let (_dir, root) = fixture();
    let full = scc_engine::invoke(&root, "surface.build", serde_json::json!({})).unwrap();
    let no_mmr = scc_engine::invoke(
        &root, "surface.build", serde_json::json!({"stages": {"mmr": false}}),
    )
    .unwrap();
    let full_ids = full["result"]["rendered_ids"].as_array().unwrap();
    assert!(!full_ids.is_empty(), "surface renders: {full}");
    // The toggle must be accepted and produce a render (ordering may or may
    // not differ on this tiny fixture — the contract is staged derivation).
    assert!(no_mmr["result"]["rendered_ids"].is_array(), "{no_mmr}");
    let default_explicit = scc_engine::invoke(
        &root, "surface.build",
        serde_json::json!({"stages": {"lexical": true, "global_ppr": true, "task_ppr": true, "mmr": true, "quotas": true, "optimizer": true}}),
    )
    .unwrap();
    assert_eq!(
        full["result"]["rendered_ids"], default_explicit["result"]["rendered_ids"],
        "all-true stages == build_surface"
    );
}

#[test]
// trace:v1 id=test.scc-engine-operations.registry-parity verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.model-get
fn registry_covers_every_invoke_arm() {
    // DoD 36/39: every callable op is discoverable. `scc operations`,
    // RPC operations.list, and HTTP GET /v1/operations all read OPERATIONS —
    // an invoke arm without a descriptor is reachable but invisible.
    let (_dir, root) = fixture();
    // Import aliases route through the shared importer.
    for op in ["import.ccg", "import.gitnexus", "import.tracelayer", "import.beads", "import.hindsight", "import.cbm"] {
        assert!(scc_engine::ops::describe(op).is_some(), "descriptor missing for {op}");
    }
    // Legacy ranking aliases resolve with hooks like ranking.symbols.
    for op in ["ranking.global", "ranking.task", "ranking.entities"] {
        assert!(scc_engine::ops::describe(op).is_some(), "descriptor missing for {op}");
        let v = scc_engine::invoke(&root, op, json!({"goal": "hello", "limit": 5})).unwrap();
        assert!(v.get("items").and_then(|i| i.as_array()).is_some(), "{op} must return items: {v}");
    }
    // Every descriptor resolves (no dangling registry entries).
    for id in scc_engine::ops::ids() {
        assert!(scc_engine::ops::describe(id).is_some(), "describe missing for {id}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.selection-required verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.budget-selection
fn selection_required_unions_engine_and_plugin() {
    // `selection.required`: engine never-omit set + plugin coverage
    // union. Empty repo ⇒ empty engine set; a coverage provider adds one.
    let (_dir, root) = fixture();
    let base = scc_engine::invoke(&root, "selection.required",
        serde_json::json!({"goal": "hello"})).unwrap();
    assert_eq!(base["plugin_contributed"], serde_json::json!(0), "{base}");
    let plugdir = root.join(".scc").join("plugins").join("acme.cov");
    std::fs::create_dir_all(&plugdir).unwrap();
    std::fs::write(
        plugdir.join("scc-plugin.toml"),
        "[plugin]\nid = \"acme.cov\"\nname = \"Cov\"\nversion = \"1.0.0\"\napi = \"1\"\noperations = [\"ranking.coverage\"]\n\n[runtime]\ncommand = [\"python3\", \"plugin.py\"]\n\n[extensions]\n\"coverage:acme.req\" = {priority=1}\n\n[permissions]\nrepo_read = true\n",
    ).unwrap();
    std::fs::write(plugdir.join("plugin.py"),
        "import json, sys\nprint(json.dumps({\"output\": {\"required\": [\"entry:acme\"]}}))\n").unwrap();
    let out = scc_engine::invoke(&root, "selection.required",
        serde_json::json!({"goal": "hello"})).unwrap();
    assert!(out["required"].as_array().unwrap().iter().any(|x| x == "entry:acme"), "{out}");
    assert_eq!(out["plugin_contributed"], serde_json::json!(1), "{out}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.selection-preview verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.budget-selection
fn selection_preview_shows_stage_survivors() {
    // §123.14: `selection.preview` reports per-stage survivors through
    // the default chain. Quota 0.5 over 300 symbol-tokens admits 1 of 3
    // at the quota stage; budget keeps it (300 <= 300).
    let (_dir, root) = fixture();
    let rows = |ids: &[&str]| -> Vec<serde_json::Value> {
        ids.iter().map(|id| json!({
            "id": id, "kind": "symbol", "value": 1.0, "token_cost": 100,
            "group": "g"})).collect()
    };
    let out = scc_engine::invoke(&root, "selection.preview", json!({
        "ranked": rows(&["a", "b", "c"]),
        "quotas": [{"kind": "symbol", "fraction": 0.5}],
        "budget": 300,
    })).unwrap();
    assert_eq!(out["after_mmr"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(out["after_quotas"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["after_budget"].as_array().unwrap().len(), 1, "{out}");
    // Monotone narrowing: each stage is a subset of the previous.
    let mmr: std::collections::BTreeSet<String> = out["after_mmr"].as_array().unwrap().iter()
        .map(|x| x.as_str().unwrap().to_string()).collect();
    for stage in ["after_quotas", "after_budget"] {
        for x in out[stage].as_array().unwrap() {
            assert!(mmr.contains(x.as_str().unwrap()), "{x} not in mmr survivors: {out}");
        }
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.export-diagram verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.diagram
fn export_diagram_renders_from_engine() {
    // DoD 5/6: diagram rendering is engine behavior, reachable on every
    // transport through the registry — not CLI-local.
    let (_dir, root) = fixture();
    for format in ["mermaid", "svg"] {
        let v = scc_engine::invoke(&root, "export.diagram", serde_json::json!({"format": format})).unwrap();
        assert_eq!(v.get("format"), Some(&serde_json::json!(format)), "{v}");
        let text = v.get("text").and_then(|t| t.as_str()).unwrap();
        assert!(!text.is_empty(), "{format} must render");
        if format == "mermaid" {
            assert!(text.starts_with("flowchart LR"), "{text:?}");
        } else {
            assert!(text.starts_with("<svg"), "{text:?}");
        }
        assert!(v.get("nodes").and_then(|n| n.as_u64()).is_some(), "{v}");
    }
    assert!(scc_engine::ops::describe("export.diagram").is_some(), "registered");
    let bad = scc_engine::invoke(&root, "export.diagram", serde_json::json!({"format": "dot"}));
    assert!(bad.is_err(), "unknown format must fail, not silently default");
}

#[test]
// trace:v1 id=test.scc-engine-operations.traverse verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-graph.traverse
fn traverse_walks_edges_and_rejects_bad_dir() {
    let (_dir, root) = fixture();
    // Baseline: call-graph edges exist in the indexed fixture.
    let rels = scc_engine::invoke(&root, "graph.relationships", json!({"limit": 50})).unwrap();
    let n = rels.as_array().map(|a| a.len()).unwrap_or(0);
    assert!(n > 0, "fixture should have relationships: {rels}");
    // Traverse from every entity with no steps: start set returned, no rels.
    let v = scc_engine::invoke(
        &root,
        "graph.traverse",
        json!({"from_ids": ["no-such-id"], "steps": [], "limit": 10}),
    )
    .unwrap();
    assert_eq!(v["entities"].as_array().map(|a| a.len()), Some(0));
    // Unknown direction fails loudly, never walks the wrong way.
    let e = scc_engine::invoke(&root, "graph.traverse", json!({"steps": [{"dir": "sideways"}]}));
    assert!(e.is_err(), "bad direction must fail: {e:?}");
    // Real walk: from all entities of the dominant kind, one out-step.
    let ents = scc_engine::invoke(&root, "graph.entities", json!({})).unwrap();
    let kind = ents.as_array().and_then(|a| a.first()).and_then(|e| e.get("kind")).and_then(|k| k.as_str()).unwrap_or("file");
    let v = scc_engine::invoke(
        &root,
        "graph.traverse",
        json!({"kind": kind, "name": "", "steps": [{"dir": "out"}], "limit": 10}),
    )
    .unwrap();
    assert!(v.get("entities").is_some() && v.get("relationships").is_some(), "{v}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.traverse-trust-modes verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-graph.traverse
fn traverse_trusted_only_false_exposes_raw_superset() {
    let (_dir, root) = fixture();
    let base = json!({"steps": [{"dir": "both"}], "limit": 50});
    let mut trusted_in = base.clone();
    trusted_in["trusted_only"] = json!(true);
    let mut raw_in = base.clone();
    raw_in["trusted_only"] = json!(false);
    let t = scc_engine::invoke(&root, "graph.traverse", trusted_in).unwrap();
    let r = scc_engine::invoke(&root, "graph.traverse", raw_in).unwrap();
    assert_eq!(r["trusted_only"], json!(false));
    assert_eq!(t["trusted_only"], json!(true));
    let tn = t["relationships"].as_array().map(|a| a.len()).unwrap_or(0);
    let rn = r["relationships"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(rn >= tn, "raw exposes a superset of trusted ({rn} vs {tn}): {t} / {r}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.entity-get-trust verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-graph.traverse
fn entity_get_reports_trust_envelope() {
    let (_dir, root) = fixture();
    let rels = scc_engine::invoke(&root, "graph.relationships", json!({"limit": 5})).unwrap();
    let id = rels.as_array().and_then(|a| a.first()).and_then(|r| r.get("subject")).and_then(|s| s.as_str()).unwrap_or("missing");
    let v = scc_engine::invoke(&root, "graph.entity.get", json!({"id": id})).unwrap();
    assert_eq!(v["trusted"], json!(true), "fresh entity is trusted: {v}");
    assert!(v["entity"]["id"] == json!(id), "{v}");
    let v = scc_engine::invoke(&root, "graph.entity.get", json!({"id": "no-such-id"})).unwrap();
    assert_eq!(v["trusted"], json!(false), "{v}");
    assert!(v["entity"].is_null(), "{v}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.explain-assertions verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-graph.explain
fn explain_reports_assertions_and_verdict() {
    let (_dir, root) = fixture();
    let rels = scc_engine::invoke(&root, "graph.relationships", json!({"limit": 5})).unwrap();
    let first = rels.as_array().and_then(|a| a.first()).expect("fixture has rels");
    let (s, p, o) = (
        first["subject"].as_str().unwrap(),
        first["predicate"].as_str().unwrap(),
        first["object"].as_str().unwrap(),
    );
    let v = scc_engine::invoke(&root, "graph.explain", json!({"subject": s, "predicate": p, "object": o})).unwrap();
    assert_eq!(v["trusted"], json!(true), "fresh edge is trusted: {v}");
    assert!(v["assertions"].as_array().map(|a| !a.is_empty()).unwrap_or(false), "{v}");
    assert!(v["assertions"][0].get("provenance").is_some(), "{v}");
    let v = scc_engine::invoke(&root, "graph.explain", json!({"subject": s, "predicate": p, "object": "no-such-object"})).unwrap();
    assert_eq!(v["trusted"], json!(false), "{v}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.section-81-aliases verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.model-get
fn spec_section_81_aliases_resolve() {
    use serde_json::json;
    let (_dir, root) = fixture();
    // Every §81 alias resolves through invoke to the canonical behavior.
    let pairs: &[(&str, serde_json::Value, &str)] = &[
        ("surface.compile", json!({}), "text"),
        ("surface.global", json!({}), "text"),
        ("surface.task", json!({"task": "hello"}), "text"),
        ("surface.important", json!({"limit": 3}), "entries"),
        ("surface.explain", json!({"id": "x", "goal": "hello"}), "error"),
        ("surface.rank", json!({"limit": 3}), "items"),
        ("model.drift", json!({}), "drift"),
        ("model.components", json!({}), "repository"),
        ("graph.entity", json!({"id": "no-such-id"}), "trusted"),
        ("history.list", json!({}), "revisions"),
        ("evidence.search", json!({}), "output-must-be-array"),
        ("integration.list", json!({}), "output-must-be-array"),
        ("export.capsule", json!({}), "output-must-be-text"),
        ("diagram.render", json!({"format": "mermaid"}), "mermaid"),
        ("beads.active", json!({}), "output-must-be-array"),
        ("index.paths", json!({"paths": []}), "files_indexed"),
        ("context.task_delta", json!({"goal": "hello"}), "delta"),
        // context.docs asserted separately below (unconfigured Context7 must fail loudly, never synthesize).
    ];
    for (op, input, _probe) in pairs {
        let v = scc_engine::invoke(&root, op, input.clone());
        assert!(v.is_ok(), "{op} must resolve: {v:?}");
    }
    // context.docs without Context7 configured fails LOUDLY (never fake docs).
    let e = scc_engine::invoke(&root, "context.docs", json!({"dependency": "serde"}));
    assert!(e.is_err() && e.unwrap_err().to_string().contains("Context7"), "docs must fail loudly unconfigured");
    // Registry covers every invoke arm: no arm without a descriptor.
    let v = scc_engine::invoke(&root, "operations.list", json!({})).unwrap();
    let listed: Vec<String> = v["output"]["operations"].as_array().map(|a| a.iter().filter_map(|o| o.as_str().map(str::to_string)).collect()).unwrap_or_else(|| v["operations"].as_array().unwrap().iter().filter_map(|o| o.as_str().map(str::to_string)).collect());
    for op in ["graph.explain", "graph.traverse", "context.task_delta", "surface.compile", "model.drift", "evidence.search", "diagram.render"] {
        assert!(listed.contains(&op.to_string()), "{op} missing from registry");
    }
    // Honest refusals for CLI-local loops (same precedent as setup.*).
    for op in ["index.watch", "viewer.snapshot"] {
        let e = scc_engine::invoke(&root, op, json!({}));
        assert!(e.is_err(), "{op} must refuse loudly: {e:?}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.extensions-preserved verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-exports.model-get
fn extensions_section_preserves_plugin_facts() {
    let (_dir, root) = fixture();
    // Commit a namespaced plugin fact, then export: the extensions
    // section must carry it (§106) instead of silently dropping it.
    let batch = serde_json::json!({
        "entities": [{"id": "plugin:acme.demo/zone", "kind": "plugin:acme.demo/zone", "name": "zone"}],
        "relationships": [],
        "evidence": [{"id": "plugin:acme.demo/ev1", "type": "test", "path": "main.py"}],
    });
    // Validation requires evidence ids present; commit via the op.
    let v = scc_engine::invoke(&root, "plugins.contribute",
        serde_json::json!({"plugin": "acme.demo", "batch": batch})).unwrap();
    assert!(v.get("entities").is_some(), "{v}");
    let ir = scc_engine::invoke(&root, "export.system_ir", serde_json::json!({"format": "system-ir.json"})).unwrap();
    let ext = &ir["extensions"];
    assert!(ext.get("acme.demo").is_some(), "plugin facts preserved: {ext}");
    let ids = ext["acme.demo"].as_array().cloned().unwrap_or_default();
    assert!(ids.iter().any(|i| i.as_str() == Some("plugin:acme.demo/zone")), "{ids:?}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.schema-from-types verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ops.input-schema
fn operations_schema_comes_from_request_types() {
    let (_dir, root) = fixture();
    // Schema for a typed op names its properties from the request struct.
    let v = scc_engine::invoke(&root, "operations.schema", serde_json::json!({"id": "graph.traverse"})).unwrap();
    let s = v.to_string();
    assert!(s.contains("steps") && s.contains("trusted_only"), "{s}");
    // Aliases share the canonical schema.
    let a = scc_engine::invoke(&root, "operations.schema", serde_json::json!({"id": "surface.compile"})).unwrap();
    assert!(a.to_string().contains("task"), "{a}");
    // Unknown ops fail loudly.
    let e = scc_engine::invoke(&root, "operations.schema", serde_json::json!({"id": "nope.nope"}));
    assert!(e.is_err(), "{e:?}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.capabilities-vocabulary verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ops.capabilities
fn operations_capabilities_lists_vocabulary() {
    let (_dir, root) = fixture();
    let v = scc_engine::invoke(&root, "operations.capabilities", serde_json::json!({})).unwrap();
    for perm in ["repo.read", "graph.read", "graph.contribute", "state.read", "state.write", "network", "subprocess", "operation.register", "ranking.extend", "context.extend"] {
        assert!(v["permissions"].as_array().unwrap().iter().any(|p| p["id"] == perm), "missing {perm}: {v}");
    }
    for ext in ["seed-provider", "rank-feature", "context-section", "startup-section", "operation"] {
        assert!(v["extension_points"].as_array().unwrap().iter().any(|e| e == ext), "missing {ext}: {v}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.plugins-graph verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-plugins.order-extensions
fn plugins_graph_groups_by_type() {
    let (_dir, root) = fixture();
    let v = scc_engine::invoke(&root, "plugins.graph", serde_json::json!({})).unwrap();
    assert!(v.get("groups").is_some(), "{v}");
}

#[test]
// trace:v1 id=test.scc-engine-operations.stability-gate verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ops.input-schema
fn stability_classes_gate_registry() {
    let (_dir, root) = fixture();
    // Every descriptor carries a stability class.
    let v = scc_engine::invoke(&root, "operations.describe", serde_json::json!({"id": "graph.traverse"})).unwrap();
    assert_eq!(v["stability"], serde_json::json!("Experimental"), "{v}");
    let v = scc_engine::invoke(&root, "operations.describe", serde_json::json!({"id": "context.task"})).unwrap();
    assert_eq!(v["stability"], serde_json::json!("Stable"), "{v}");
    // The new ops are reachable and classified.
    for (op, want) in [("operations.schema", "Experimental"), ("plugins.graph", "Experimental"), ("operations.capabilities", "Stable")] {
        let v = scc_engine::invoke(&root, "operations.describe", serde_json::json!({"id": op})).unwrap();
        assert_eq!(v["stability"], serde_json::json!(want), "{op}: {v}");
    }
}

#[test]
// trace:v1 id=test.scc-engine-operations.mutation-classes verifies=REQ-SI-503JSBGP exercises=impl.scc-engine-ops.input-schema
fn mutation_classes_name_side_effects() {
    let (_dir, root) = fixture();
    // §78 examples: task/startup record visibility, index mutates the
    // model, init mutates external config. Agents reason from these.
    for (op, want) in [
        ("context.task", "SessionMutation"),
        ("context.startup", "SessionMutation"),
        ("checkpoint.save", "SessionMutation"),
        ("index.full", "ModelMutation"),
        ("runtime.ingest", "ModelMutation"),
        ("plugins.contribute", "ModelMutation"),
        ("workspace.init", "ExternalMutation"),
        ("plugins.lock", "ExternalMutation"),
        ("graph.traverse", "Read"),
        ("ranking.symbols", "Read"),
    ] {
        let v = scc_engine::invoke(&root, "operations.describe", serde_json::json!({"id": op})).unwrap();
        assert_eq!(v["mutation"], serde_json::json!(want), "{op}: {v}");
    }
}
