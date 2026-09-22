//! Engine integrations: adapter listing + doctor report VALUES.
//!
//! The report derivation (registry rows, contribution signals, handshake
//! policy) moves verbatim from `scc-cli` commands.rs; transports render it.
//! Byte-identical: same rows, same warnings, same order.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// trace:exempt reason=internal-detail
fn adapter_scope(name: &str) -> &'static str {
    match name {
        "context7" => "network+subprocess(npx)",
        "serena" | "beads" | "cbm" | "hindsight" | "scip" | "gitnexus" | "narsil" => "filesystem",
        _ => "filesystem",
    }
}

// trace:exempt reason=internal-detail
pub fn list(root: &Path) -> crate::Result<Vec<(String, &'static str)>> {
    use scc_indexer::adapters::IntegrationCategory;
    let config = crate::workspace::load_config(root)?;
    let enabled = |key: Option<&str>| -> bool {
        match key {
            None => true,
            Some("serena") => config.integrations.serena,
            Some("beads") => config.integrations.beads,
            Some("hindsight") => config.integrations.hindsight,
            Some("gitnexus") => config.integrations.gitnexus,
            Some("context7_command") => !config.integrations.context7_command.is_empty(),
            Some(_) => false,
        }
    };
    Ok(scc_indexer::adapters::integration_registry()
        .into_iter()
        .filter(|d| {
            d.category != IntegrationCategory::CompatibilityOnly
                && d.category != IntegrationCategory::InternalPass
                && d.category != IntegrationCategory::AgentIntegration
                && enabled(d.config_key)
        })
        .map(|d| (d.id.to_string(), adapter_scope(d.id)))
        .collect())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct DoctorRow {
    pub id: String,
    pub category: String,
    pub mode: String,
    pub implemented: bool,
    pub configured: bool,
    pub reachable: Option<bool>,
    pub contributing: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct DoctorAgent {
    pub id: String,
    pub present: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct DoctorReport {
    pub scc_version: String,
    pub revision: String,
    pub stale_files: usize,
    pub dangling_edges: usize,
    pub stats: HashMap<String, u64>,
    pub integrations: Vec<DoctorRow>,
    pub agents: Vec<DoctorAgent>,
    pub warnings: u32,
}

// trace:exempt reason=internal-detail
pub fn doctor_report(
    store: &scc_store::Store,
    config: &scc_indexer::Config,
    root: &Path,
    deep: bool,
    network: bool,
) -> crate::Result<DoctorReport> {
    use scc_indexer::adapters::integration_registry;
    let stale = crate::workspace::stale_paths(store).unwrap_or_default();
    let mut rows: Vec<DoctorRow> = Vec::new();
        let mut warnings: u32 = 0;

        // Evidence already in the graph, by extractor tag (the honest
        // contribution signal): native facts carry extractor `scc-native`;
        // TraceLayer facts carry requirement/decision/implementation/test/work
        // entity kinds (the importer emits no extractor tag); beads/cbm/
        // hindsight/gitnexus carry their kinds.
        let ev_by_extractor: std::collections::BTreeMap<String, u64> = store
            .all_evidence()
            .map(|evs| {
                let mut m = std::collections::BTreeMap::new();
                for e in evs {
                    if let Some(x) = e.extractor.as_deref() {
                        *m.entry(x.to_string()).or_insert(0) += 1;
                    }
                }
                m
            })
            .unwrap_or_default();
        let count_kind = |kind: &str| -> u64 {
            store.entities_by_kind(kind).map(|v| v.len() as u64).unwrap_or(0)
        };
        let on_path = |bin: &str| -> bool {
            std::env::var_os("PATH")
                .map(|p| {
                    std::env::split_paths(&p).any(|d| {
                        d.join(bin).is_file() || d.join(format!("{bin}.exe")).is_file()
                    })
                })
                .unwrap_or(false)
        };

        // CORE section facts (not registry rows).
        let scc_version = env!("CARGO_PKG_VERSION");
        let stats = store.stats().unwrap_or_default();
        let rev = store
            .snapshot_status()
            .ok()
            .flatten()
            .map(|(s, _)| s.revision.to_string())
            .unwrap_or_else(|| "unindexed".into());
        let dangling = scc_graph::RealityGraph::load(store)
            .map(|g| {
                g.all_rels()
                    .iter()
                    .filter(|r| {
                        let known = |id: &str| {
                            g.entities.contains_key(id)
                                || id.contains("/external_api/")
                                || id.contains("/component/")
                                || id.contains("/flow/")
                                || id.contains("/invariant/")
                        };
                        !known(&r.subject) || !known(&r.object)
                    })
                    .count()
            })
            .unwrap_or(0);

        for d in integration_registry() {
            let (configured, reachable, contributing, detail) = match d.id {
                "native" => {
                    let n: u64 = stats.get("entities").copied().unwrap_or(0);
                    (true, None, n > 0, format!("{n} entities in graph"))
                }
                "scip" | "ccg" | "cbm" => {
                    // On-demand file import, always available, needs no config:
                    // an availability note, never a warning.
                    (false, None, false, format!("importer available (on-demand `scc import {}`)", d.id))
                }
                "gitnexus" => {
                    let n = count_kind("symbol")
                        + ev_by_extractor.get("gitnexus").copied().unwrap_or(0);
                    let cli = on_path("gitnexus");
                    (
                        config.integrations.gitnexus,
                        None,
                        n > 0 && config.integrations.gitnexus,
                        format!(
                            "mode: file import; configured: {}; CLI detected: {}; symbol-ish evidence: {n}",
                            config.integrations.gitnexus, cli
                        ),
                    )
                }
                "tracelayer" => {
                    let n = count_kind("requirement")
                        + count_kind("decision")
                        + count_kind("implementation")
                        + count_kind("test")
                        + count_kind("work");
                    (
                        true,
                        None,
                        n > 0,
                        format!("{n} trace facts (requirement/decision/implementation/test/work); last import: revision {rev}"),
                    )
                }
                "beads" => {
                    let n = count_kind("task");
                    (
                        config.integrations.beads,
                        None,
                        n > 0 && config.integrations.beads,
                        format!("task-state entities: {n}; configured: {}", config.integrations.beads),
                    )
                }
                "hindsight" => {
                    let n = count_kind("lesson");
                    (
                        config.integrations.hindsight,
                        None,
                        n > 0 && config.integrations.hindsight,
                        format!("lessons in graph: {n}; bank: .scc/lessons.jsonl"),
                    )
                }
                "context7" => {
                    let cfg = !config.integrations.context7_command.is_empty();
                    let reach = if network {
                        // Explicit opt-in only: never probed by default.
                        Some(config.integrations.context7_command.contains("context7"))
                    } else {
                        None
                    };
                    if cfg && reach == Some(false) {
                        warnings += 1;
                    }
                    (
                        cfg,
                        reach,
                        false,
                        if cfg {
                            "configured; handshake only under --network (never auto-downloads)".into()
                        } else {
                            "disabled (empty context7_command)".into()
                        },
                    )
                }
                "lsp-pyright" => {
                    let bin = on_path("pyright") || on_path("basedpyright");
                    let reach = if deep {
                        // --deep may handshake locally; default only reports
                        // the binary presence (offline, read-only).
                        Some(bin && std::process::Command::new("pyright").arg("--version").output().map(|o| o.status.success()).unwrap_or(false))
                    } else {
                        None
                    };
                    if !bin {
                        warnings += 1;
                    }
                    (true, reach, bin, if bin { "binary on PATH (resolver available)".into() } else { "not installed".into() })
                }
                "lsp-tsserver" => {
                    let bin = on_path("tsserver") || on_path("typescript-language-server");
                    if !bin {
                        warnings += 1;
                    }
                    (true, if deep { Some(bin) } else { None }, bin, if bin { "binary on PATH (resolver available)".into() } else { "not installed".into() })
                }
                "runtime" => {
                    (false, None, false, "ingestion source available (`scc ingest`)".into())
                }
                "serena" => {
                    // Compatibility-only: presence is coexistence info, never
                    // a warning either way.
                    (
                        false,
                        None,
                        false,
                        "compatibility only; no SCC evidence adapter (coexistence/exact-source workflow)".into(),
                    )
                }
                "configrefs" | "failures" => (true, None, true, "internal post-pass; always runs at index".into()),
                _ => (false, None, false, "unknown integration".into()),
            };
            // Unconfigured-but-capable importers are availability notes, not
            // warnings; missing optionals (pyright binary) and failed
            // handshakes warn; strict promotes every non-contributing
            // configured row to a warning at report time (counted below).
            if matches!(d.id, "gitnexus" | "beads" | "hindsight") && configured && !contributing {
                warnings += 1;
            }
            rows.push(DoctorRow {
                id: d.id.to_string(),
                category: d.category.as_str().into(),
                mode: d.mode.to_string(),
                implemented: true,
                configured,
                reachable,
                contributing,
                detail,
            });
        }

        // Agent integrations: presence probes over install artifacts (local,
        // offline). Hermes installs outside the repo, so absence is a note.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let agent_rows: Vec<DoctorAgent> = vec![
            DoctorAgent { id: "omp".into(), present: root.join(".omp").is_dir() || root.join("plugins/omp/scc").is_dir(), detail: "repo .omp/ or plugin dir".into() },
            DoctorAgent { id: "claude-code".into(), present: root.join("plugins/claude").is_dir(), detail: "repo plugin dir".into() },
            DoctorAgent { id: "codex".into(), present: root.join("AGENTS.md").is_file(), detail: "AGENTS.md present".into() },
            DoctorAgent { id: "opencode".into(), present: root.join("plugins/opencode").is_dir(), detail: "repo plugin dir".into() },
            DoctorAgent { id: "hermes".into(), present: home.as_ref().map(|h| h.join(".hermes").is_dir()).unwrap_or(false) || root.join("plugins/hermes").is_dir(), detail: "home or repo plugin".into() },
            DoctorAgent { id: "mcp-server".into(), present: root.join(".mcp.json").is_file() || root.join("opencode.json").is_file(), detail: "MCP config present".into() },
        ];
    Ok(DoctorReport {
        scc_version: scc_version.into(),
        revision: rev,
        stale_files: stale.len(),
        dangling_edges: dangling,
        stats,
        integrations: rows,
        agents: agent_rows,
        warnings,
    })
}
