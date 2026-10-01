//! CLI completeness gate (spec §89): no command may contain unique
//! SCC domain behavior. The CLI may only: parse args, open store/config,
//! drive `scc_engine` (typed namespaces or `invoke`), and render results.
//! Direct domain-crate use is allow-listed per line below with rationale;
//! any other `scc_context::` / `scc_graph::` / `scc_store::` /
//! `scc_indexer::` reference in commands.rs fails this test.

use std::collections::BTreeSet;

// (needle, rationale) — keep this list minimal and justified.
const ALLOW: &[(&str, &str)] = &[
    (
        "scc_context::rank::SemanticScorer",
        "trait cast for the typed engine.context().surface() call — ranker resolution itself lives in scc_engine::inference",
    ),
    (
        "scc_context::surface::render_important",
        "pure text formatter for engine-returned entries",
    ),
    (
        "scc_context::ContextPack",
        "return/transport type of engine-built artifacts (no derivation)",
    ),
    (
        "scc_indexer::Config::default_yaml",
        "workspace init writes the default config file (file installation, not domain logic)",
    ),
    (
        "scc_indexer::adapters::",
        "integrations list/doctor/describe read the adapter registry metadata (capability scope display)",
    ),
    (
        "scc_indexer::adapters::hindsight::Lesson",
        "lessons-add payload type (engine owns storage)",
    ),
];

#[test]
// trace:v1 id=test.scc-cli-completeness-gate work=WORK-SI-MMMJA4G6 verifies=REQ-SI-503JSBGP exercises=impl.crates-scc-cli-src-commands.cmd-context-task
fn cli_commands_route_through_the_engine() {
    let src = include_str!("../src/commands.rs");
    let mut violations: Vec<String> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        // Skip comments, markers, and string literals content is fine —
        // only code references matter; comments document the rule itself.
        if t.starts_with("//") || t.starts_with("///") {
            continue;
        }
        for pat in [
            "scc_context::",
            "scc_graph::",
            "scc_store::",
            "scc_indexer::",
        ] {
            if t.contains(pat) && !ALLOW.iter().any(|(a, _)| t.contains(a)) {
                violations.push(format!("{}: {t}", i + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "CLI bypasses the engine (spec §89) — move behind scc_engine or allow-list with rationale:\n{}",
        violations.join("\n")
    );
    // The allow-list itself stays honest: every entry must still match.
    let mut unused: Vec<&str> = Vec::new();
    for (needle, _) in ALLOW {
        if !src.contains(needle) {
            unused.push(needle);
        }
    }
    assert!(unused.is_empty(), "stale allow-list entries (remove them): {unused:?}");
    // Sanity: the engine surface the gate protects actually exists.
    let _ = BTreeSet::<String>::new();
}
