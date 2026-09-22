//! Command implementations for the `scc` CLI (docs/API_AND_INTEGRATIONS.md §4).

use crate::{config_path, load_config, open_store, scc_dir};

// trace:exempt reason=internal-detail
fn engine_err(e: scc_engine::EngineError) -> crate::CliError {
    crate::CliError::Other(e.to_string())
}
use std::io::Write;
use std::path::{Path, PathBuf};

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-init work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_init(root: &Path) -> crate::Result<()> {
    let dir = scc_dir(root);
    std::fs::create_dir_all(&dir)?;
    let cfg_path = config_path(root);
    if !cfg_path.exists() {
        std::fs::write(&cfg_path, scc_indexer::Config::default_yaml())?;
        println!("created {}", cfg_path.display());
    } else {
        println!("config exists: {}", cfg_path.display());
    }
    // create the DB so the workspace is ready
    let store = open_store(root)?;
    crate::ensure_scc_ignored(root);
    println!(
        "initialized SCC workspace for repository '{}' at {}",
        store.repo_name,
        dir.display()
    );
    println!("next: scc index");
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-index work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_index(root: &Path, quiet: bool) -> crate::Result<()> {
    let config = load_config(root)?;
    let report = scc_engine::index::full(root, &config).map_err(engine_err)?;
    if !quiet {
        println!(
            "indexed {} file(s) ({} changed, {} added, {} removed, {} failed) in {:.2}s",
            report.indexed,
            report.changed,
            report.added,
            report.removed,
            report.failed,
            report.duration_ms as f64 / 1000.0
        );
        println!("analysis_quality: {}", report.analysis_quality.compact_line());
        let s = &report.scan_stats;
        println!(
            "files: discovered={} indexed={} ignored={} unsupported={} oversized={} unreadable={}",
            s.discovered, s.indexed, s.ignored, s.unsupported, s.oversized, s.unreadable
        );
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-index-paths work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_index_paths(root: &Path, paths: &[String], quiet: bool) -> crate::Result<()> {
    let config = load_config(root)?;
    let report = scc_engine::index::refresh_paths(root, &config, paths).map_err(engine_err)?;
    if !quiet && report.indexed > 0 {
        println!("refreshed {} file(s)", report.indexed);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-status work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_status(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    let s = scc_engine::status::status(&store).map_err(engine_err)?;
    println!("Repository: {} ({})", s.repository, s.repository_id);
    if let Some(url) = &s.remote {
        println!("Remote: {url}");
    }
    if s.indexed {
        println!("Revision: {}", s.revision);
        if let Some(b) = &s.branch {
            println!("Branch: {b}");
        }
        println!("Indexed at: {}", s.indexed_at.as_deref().unwrap_or(""));
        let mut keys: Vec<&String> = s.stats.keys().collect();
        keys.sort();
        for k in keys {
            println!("{k}: {}", s.stats[k]);
        }
        if s.freshness == "CURRENT" {
            println!("freshness: CURRENT — model matches working tree");
        } else {
            println!(
                "freshness: STALE — {} file(s) changed since index (run `scc index`)",
                s.stale_count
            );
            for f in &s.stale_files {
                println!("  {f}");
            }
        }
        if let Some(raw) = &s.analysis_quality {
            match serde_json::from_str::<scc_core::AnalysisQuality>(raw) {
                Ok(q) => println!("analysis_quality: {}", q.compact_line()),
                Err(_) => println!("analysis_quality: {raw}"),
            }
        }
        if let Some(v) = &s.scan_stats {
            // Counts only, recorded at index time; live staleness is
            // reported above from the working-tree scan.
            let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            println!(
                "files: discovered={} indexed={} ignored={} unsupported={} oversized={} unreadable={}",
                n("discovered"),
                n("indexed"),
                n("ignored"),
                n("unsupported"),
                n("oversized"),
                n("unreadable")
            );
        }
    } else {
        println!("not indexed yet — run `scc index`");
    }
    Ok(())
}

/// `scc languages` — generated support matrix. Never hand-maintain a
/// second list in CLI copy.
// trace:v1 id=impl.scc.cli.languages work=WORK-ripwire-lessons-phase1 satisfies=REQ-language-support-matrix
pub fn cmd_languages() -> crate::Result<()> {
    print!("{}", scc_core::support_matrix_markdown());
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-overview work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_overview(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().overview().map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}
// trace:v1 id=impl.scc.cli work=WORK-SCC-001 satisfies=REQ-SCC-API

/// `scc atlas [--budget N] [--json]` — the full System Atlas.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-atlas work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_atlas(
    root: &Path,
    budget: Option<usize>,
    json: bool,
    full: bool,
    unbounded: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().atlas(budget, full, unbounded).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

/// `scc context startup [--budget N]` — the Wave 14 startup artifact:
/// the Atlas + Surface fusion, deterministic per epoch (prompt-cache
/// stable). Records what the session just showed in the context ledger so
/// task deltas suppress already-visible APIs.
// trace:exempt reason=internal-detail
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-startup work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_startup(root: &Path, budget_tokens: Option<usize>) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let (_startup, text) = engine.context().startup(&scc_api::StartupRequest { budget: budget_tokens }).map_err(engine_err)?;
    print!("{text}");
    Ok(())
}

/// `scc surface [--task "<goal>"] [--budget N] [--explain]` — the System
/// Surface Map: the actual callable API layer, global or task-personalized.
/// BOTH modes run the ONE authoritative service — `build_surface` (Global
/// or Task) — so the CLI, MCP, hermes, and the SDKs render the same
/// artifact for the same request (no parallel pipelines). The rendered
/// entries are recorded in the context ledger.
// trace:exempt reason=internal-detail
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-surface work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_surface(
    root: &Path,
    task: Option<&str>,
    budget: Option<usize>,
    explain: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let tokens = budget.unwrap_or(scc_core::ContextBudget::default().surface);
    // Semantic scorer (SCC-071): wired in when the embed_cli rankers are
    // available (inference.enabled + remote-model policy) and the surface
    // is task-personalized (a scorer rates entities against a goal; the
    // global surface has no goal). Disabled embeddings → None, and the
    // pipeline explicitly redistributes the 10% semantic share.
    let scorer = match task {
        Some(goal) => {
            let (scorer, _reranker) = scc_engine::inference::rankers(&store, &config, goal);
            scorer
        }
        None => None,
    };
    let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
        scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
    let req = scc_api::SurfaceRequest { task: task.map(|s| s.to_string()), budget: Some(tokens), explain, stages: None };
    let (_result, text) = engine.context().surface(&req, semantic).map_err(engine_err)?;
    print!("{text}");
    Ok(())
}

/// `scc important [--limit N] [--component C] [--task G] [--json]` —
/// the fast "where do I pay attention first" answer (audit item 3). Global
/// mode ranks by architectural centrality (global PPR + badges); --task
/// re-ranks by task PPR and renders TASK-CRITICAL SYMBOLS. --component
/// filters to one component substring. No new MCP tool: the section also
/// ships inside startup/task context.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-important work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_important(
    root: &Path,
    limit: usize,
    component: Option<&str>,
    task: Option<&str>,
    json: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let (entries, tasked) = engine.context().important(limit, component, task).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    print!("{}", scc_context::surface::render_important(&entries, tasked));
    Ok(())
}

/// `scc context structural --files <paths...> | --task "<goal>" [--budget N]` —
/// the Structural Source product surface (fixwave Item 7): the per-file
/// signature/structural representation of the requested files, or of the
/// files matched to a task goal. Returns the rendered text so every
/// transport (CLI, MCP) shares one implementation.
// trace:exempt reason=internal-detail
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-structural work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_structural(
    root: &Path,
    files: &[String],
    task: Option<&str>,
    budget: Option<usize>,
) -> crate::Result<String> {
    // Registry derivation: parse + dispatch only; the engine owns the build.
    let out = scc_engine::invoke(
        root,
        "context.structural",
        serde_json::json!({"files": files, "task": task, "budget": budget}),
    )
    .map_err(engine_err)?;
    Ok(out.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
}

/// THE one complete task artifact (transport parity): the enriched task
/// pack AND its surface delta derived together, from ONE builder.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-commands.task-context-artifact work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-complete-task-context-identical-across-transports
pub struct TaskContextArtifact {
    pub pack: scc_context::ContextPack,
    /// The task-personalized Surface delta (only NEW relevant APIs vs the
    /// ledger). Empty string when the delta budget is 0.
    pub delta: String,
    /// Entry ids the delta rendered (ledger recording). ALWAYS serialized
    /// (empty array, not omitted) — the public JSON contract the SDKs type.
    #[serde(default)]
    pub delta_ids: Vec<String>,
    /// Actual token count of the complete rendered artifact:
    /// `estimate_tokens(pack.content) + estimate_tokens(delta)` computed
    /// AFTER all enrichment (Beads/Hindsight), never from stale component
    /// estimates — the accounting the SDKs and benchmarking rely on.
    #[serde(default)]
    pub token_count: usize,
}

/// Build the complete task artifact: scorer + beads + hindsight enrichment
/// for the pack, then the SAME-scorer task delta against the ledger.
/// `hook` keeps the whole focus within the §37 1500-token cap (the delta
/// gets what the pack leaves); an explicit budget caps the total; the
/// default gives the delta its own `task_delta` slice.
// trace:v1 id=impl.crates-scc-cli-src-commands.build-task-context work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-complete-task-context-identical-across-transports
pub fn build_task_context(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    hook: bool,
) -> crate::Result<TaskContextArtifact> {
    build_task_context_engine(root, goal, files, symbols, budget, hook).map_err(engine_err).map(from_engine_artifact)
}

// trace:exempt reason=internal-detail
fn from_engine_artifact(a: scc_engine::TaskContextArtifact) -> TaskContextArtifact {
    TaskContextArtifact { pack: a.pack, delta: a.delta, delta_ids: a.delta_ids, token_count: a.token_count }
}

// trace:exempt reason=internal-detail
fn build_task_context_engine(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    hook: bool,
) -> scc_engine::Result<scc_engine::TaskContextArtifact> {
    // Registry derivation: the engine owns the task build; this crate only
    // parses args and renders. One path for CLI/HTTP/MCP/SDKs (spec 2).
    let out = scc_engine::invoke(
        root,
        "context.task",
        serde_json::json!({"goal": goal, "files": files, "symbols": symbols, "budget": budget, "hook": hook}),
    )?;
    serde_json::from_value(out).map_err(|e| scc_engine::EngineError::Other(e.to_string()))
}

/// pack with scorer + beads + hindsight and its post-enrichment token
/// count. NO Surface delta, NO ContextLedger mutation, NO visibility side
/// effects. Pack-only callers (`context compress`, [`build_task_pack`])
/// MUST use this — NEVER [`build_task_context`], which also builds a delta
/// and records its rendered ids in the ledger: throwing the delta away
/// would mark Surface APIs "already visible" the agent never saw, breaking
/// the ledger invariant (ledger = content actually shown to the agent).
// trace:v1 id=impl.crates-scc-cli-src-commands.build-enriched-task-pack work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-complete-task-context-identical-across-transports
pub fn build_enriched_task_pack(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    hook: bool,
) -> crate::Result<scc_context::ContextPack> {
    // Registry derivation: the engine owns the pack build (spec 2).
    let out = scc_engine::invoke(
        root,
        "context.task_pack",
        serde_json::json!({"goal": goal, "files": files, "symbols": symbols, "budget": budget, "hook": hook}),
    )
    .map_err(engine_err)?;
    // Engine returns the pack directly (ContextPack), not wrapped.
    serde_json::from_value(out).map_err(|e: serde_json::Error| crate::CliError::Other(e.to_string()))
}

/// `scc context task <goal> [--budget N] [--json] [--hook]` — the complete
/// task focus. Text and JSON are TWO VIEWS OF ONE ARTIFACT built by
/// [`build_task_context`]: JSON carries `{pack, delta, delta_ids}`, text
/// prints pack content followed by the delta — identical derivation, no
/// transport downgrades quality.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-task work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-complete-task-context-identical-across-transports
pub fn cmd_context_task(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    json: bool,
    hook: bool,
) -> crate::Result<()> {
    if hook {
        // Wave 2 (§37): UserPromptSubmit injects a task focus only when
        // context.inject_task_focus is enabled; otherwise silent no-op.
        let config = load_config(root)?;
        if !config.context.inject_task_focus {
            return Ok(());
        }
    }
    let artifact = build_task_context(root, goal, files, symbols, budget, hook)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&artifact)?);
    } else {
        print!("{}", artifact.pack.content);
        print!("\n{}", artifact.delta);
    }
    Ok(())
}


/// half built by the pure [`build_enriched_task_pack`] — NO delta, NO
/// ledger side effects. New transports should call
/// [`build_task_context`] directly so the delta ships with the pack.
// trace:exempt reason=internal-detail
pub fn build_task_pack(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
) -> crate::Result<scc_context::ContextPack> {
    build_enriched_task_pack(root, goal, files, symbols, budget, false)
}
/// The complete task artifact as JSON — the same derivation as CLI text
/// (`{pack, delta, delta_ids}`); serialization is the only difference.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-task-json work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching satisfies=REQ-complete-task-context-identical-across-transports
pub fn cmd_context_task_json(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
) -> crate::Result<String> {
    Ok(serde_json::to_string_pretty(&build_task_context(
        root, goal, files, symbols, budget, false,
    )?)?)
}

/// `scc context docs <dependency>` — external library docs via Context7
/// (labeled external; never mixed with repository facts).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-docs work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_docs(root: &Path, dependency: &str) -> crate::Result<()> {
    print!("{}", scc_engine::invoke(root, "context.external_docs", serde_json::json!({"dependency": dependency})).map_err(engine_err)?);
    Ok(())
}

/// Subagent context policy (SCC-107, docs/API_AND_INTEGRATIONS.md §5):
/// a narrower, tighter-budget task pack with explicit scope boundaries so
/// delegated agents start from the same system model without re-deriving it.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-subagent work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_subagent(
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    json: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().subagent(goal, files, symbols, budget).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-component work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_component(root: &Path, id: &str, json: bool, unbounded: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().component(&scc_api::DetailRequest { id: id.into(), unbounded }).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-context-flow work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_context_flow(root: &Path, id: &str, json: bool, unbounded: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().flow(&scc_api::DetailRequest { id: id.into(), unbounded }).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-impact work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_impact(
    root: &Path,
    files: &[String],
    symbols: &[String],
    diff: Option<&str>,
    json: bool,
    unbounded: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().impact(&scc_api::ImpactRequest { files: files.to_vec(), symbols: symbols.to_vec(), diff: diff.map(|s| s.to_string()), unbounded }).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print!("{}", pack.content);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-verify work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_verify(
    root: &Path,
    warnings_only: bool,
    json: bool,
    unbounded: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let stale = crate::stale_paths(&store)?;
    let engine = scc_engine::workspace::open_engine(&store, &config, stale).map_err(engine_err)?;
    let pack = engine.context().verify(unbounded).map_err(engine_err)?;
    if warnings_only {
        for w in &pack.warnings {
            println!("⚠ {w}");
        }
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
        return Ok(());
    }
    print!("{}", pack.content);
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-drift work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_drift(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let findings = scc_engine::misc::drift(&store).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        if findings.is_empty() {
            println!("no drift findings");
        }
        for f in &findings {
            println!("[{0}] {1} (#{2}, {3}): {4}", f.severity, f.kind, f.id, f.created_at, f.message);
        }
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-system work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_system(_root: &Path, members: &[std::path::PathBuf], json: bool) -> crate::Result<()> {
    let (members, all) = scc_engine::systems::stitch(members).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&all)?);
        return Ok(());
    }
    println!(
        "system: {} members, {} stitches",
        members.len(),
        all.len()
    );
    for m in &members {
        println!("member {} ({})", m.repo_id, m.root.display());
    }
    for s in &all {
        println!(
            "[{:?}/{:?}] {} ({} ends)",
            s.kind,
            s.match_kind,
            s.key,
            s.ends.len()
        );
        for e in &s.ends {
            match e.role {
                Some(role) => println!("  {} {} ({role:?})", e.repo_id, e.entity_id),
                None => println!("  {} {}", e.repo_id, e.entity_id),
            }
        }
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-history work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_history(root: &Path, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let revs = scc_engine::history::revisions(&store).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&revs)?);
        return Ok(());
    }
    if revs.is_empty() {
        println!("no graph revisions recorded");
        return Ok(());
    }
    for r in &revs {
        println!(
            "rev {} (base {}): {} entities, {} rels, {} files | src={} ext={} | {}",
            r.rev,
            r.base_rev,
            r.entity_count,
            r.rel_count,
            r.file_count,
            &r.source_hash[..r.source_hash.len().min(12)],
            r.extractor_version,
            r.created_at,
        );
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_diff(root: &Path, from: i64, to: i64, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    let d = scc_engine::history::diff(&store, from, to).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&d)?);
        return Ok(());
    }
    // Bounded human rendering: counts plus the first 20 ids per class.
    println!("diff rev {from}..{to}:");
    for (label, ids) in [
        ("added entities", &d.added_entities),
        ("removed entities", &d.removed_entities),
        ("modified entities", &d.modified_entities),
        ("added relationships", &d.added_relationships),
        ("removed relationships", &d.removed_relationships),
        ("modified relationships", &d.modified_relationships),
    ] {
        println!("  {label}: {}", ids.len());
        for id in ids.iter().take(20) {
            println!("    {id}");
        }
        if ids.len() > 20 {
            println!("    … ({} more)", ids.len() - 20);
        }
    }
    if !d.modified_kinds.is_empty() {
        let kinds: Vec<String> = d
            .modified_kinds
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect();
        println!("  modified kinds: {}", kinds.join(", "));
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-snapshot-save work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_snapshot_save(
    root: &Path,
    task: &str,
    budget: Option<usize>,
    json: bool,
) -> crate::Result<()> {
    let snap = scc_engine::misc::snapshot_save(root, task, budget).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
    } else {
        println!("snapshot {} ({} visible ids)", snap.id, snap.entity_ids.len());
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-snapshot-show work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_snapshot_show(root: &Path, id: &str) -> crate::Result<()> {
    let store = open_store(root)?;
    match scc_engine::snapshots::get(&store, id).map_err(engine_err)? {
        Some(s) => print!("{}", s.artifact),
        None => println!("no snapshot {id}"),
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-snapshot-diff work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_snapshot_diff(root: &Path, id: &str, json: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    match scc_engine::snapshots::diff(&store, id).map_err(engine_err)? {
        Some(d) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                println!(
                    "snapshot rev {} vs current rev {}: {} still valid, {} invalidated, {} modified{}",
                    d.snapshot_revision,
                    d.current_revision,
                    d.still_valid.len(),
                    d.invalidated.len(),
                    d.modified_entities.len(),
                    if d.artifact_changed { " (artifact would re-render differently)" } else { "" },
                );
                for (label, ids) in [
                    ("invalidated", &d.invalidated),
                    ("modified entities", &d.modified_entities),
                    ("changed relationships", &d.changed_relationships),
                    ("changed contracts", &d.changed_contracts),
                    ("changed state", &d.changed_state),
                    ("changed flows", &d.changed_flows),
                ] {
                    if ids.is_empty() {
                        continue;
                    }
                    println!("  {label} ({}):", ids.len());
                    for f in ids.iter().take(20) {
                        println!("    - {f}");
                    }
                    if ids.len() > 20 {
                        println!("    … ({} more)", ids.len() - 20);
                    }
                }
            }
        }
        None => println!("no snapshot {id}"),
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-export work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_export(root: &Path, format: &str) -> crate::Result<()> {
    let store = open_store(root)?;
    match format {
        "system-ir.json" => println!("{}", serde_json::to_string_pretty(&scc_engine::exports::system_ir(&store).map_err(engine_err)?)?),
        "system-ir.jsonl" => {
            let ir = scc_engine::exports::system_ir(&store).map_err(engine_err)?;
            for line in scc_engine::exports::jsonl(&ir).map_err(engine_err)? {
                println!("{line}");
            }
        }
        "ccg" => println!("{}", serde_json::to_string_pretty(&scc_engine::exports::ccg(&scc_engine::exports::system_ir(&store).map_err(engine_err)?).map_err(engine_err)?)?),
        "flow-graphs.json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.flow_graphs()?)?
            )
        }
        "capsule.md" => print!("{}", scc_engine::exports::capsule(root).map_err(engine_err)?),
        other => {
            return Err(crate::CliError::Other(format!(
                "unknown export format '{other}' (use system-ir.json, system-ir.jsonl, ccg, or capsule.md)"
            )))
        }
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-query work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
// trace:v1 id=impl.cli.query.fallback work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_query(root: &Path, query: &str, limit: usize) -> crate::Result<()> {
    let store = open_store(root)?;
    let hit = scc_engine::graph::query(&store, &scc_api::QueryRequest { query: query.into(), limit }).map_err(engine_err)?;
    println!("— entities —");
    for e in &hit.entities {
        println!("{} [{}]", e.name, e.kind);
    }
    println!("— symbols —");
    for (name, sig, kind, file) in &hit.symbols {
        println!("{name} ({kind}) {file} {sig}");
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-list-components work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_list_components(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    for c in scc_engine::graph::components(&store).map_err(engine_err)? {
        println!("{}", c.name);
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-list-flows work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_list_flows(root: &Path) -> crate::Result<()> {
    let store = open_store(root)?;
    for f in scc_engine::graph::flows(&store).map_err(engine_err)? {
        println!("{} [{}] {}", f.name, crate::flow_kind_str(&f.kind), f.trigger.unwrap_or_default());
    }
    Ok(())
}

/// `scc cochange`: print the git co-change pairs (files changed together
/// across commits) and, when an indexed store exists, enrich its components
/// with the signal.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-cochange work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_cochange(root: &Path, min_commits: u32) -> crate::Result<()> {
    let (pairs, n) = scc_engine::misc::cochange(root, min_commits).map_err(engine_err)?;
    if pairs.is_empty() {
        println!("no co-change pairs with >= {min_commits} shared commits");
    } else {
        println!("co-change pairs (>= {min_commits} shared commits):");
        for p in &pairs {
            println!("  {} <-> {} ×{}", p.a, p.b, p.commits);
        }
    }
    if n > 0 {
        println!("enriched {n} components with co-change signal");
    }
    Ok(())
}

/// scc verify --graph-invariants: structural checks for CI.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-check-invariants work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_check_invariants(root: &Path) -> crate::Result<bool> {
    let store = open_store(root)?;
    let violations = scc_engine::misc::check_invariants(&store).map_err(engine_err)?;
    for v in &violations {
        println!("{}", v.message);
    }
    Ok(violations.is_empty())
}

/// `scc ci check` (docs/DEPLOYMENT_AND_INFRA.md §3, EPIC-180 CI policies):
/// graph invariants + drift severity policy. Exits nonzero on violation.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-ci-check work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_ci_check(root: &Path, max_severity: &str) -> crate::Result<bool> {
    let store = open_store(root)?;
    let violations = scc_engine::misc::check_invariants(&store).map_err(engine_err)?;
    for v in &violations {
        println!("{}", v.message);
    }
    let (ok, lines) = scc_engine::misc::ci_check(&store, &violations, max_severity).map_err(engine_err)?;
    for l in &lines {
        // invariant lines already printed above; print only ci lines + summary
        if l.starts_with("[ci:") || *l == "ci check passed" {
            println!("{l}");
        }
    }
    Ok(ok)
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-checkpoint-save work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_checkpoint_save(root: &Path, json: bool) -> crate::Result<()> {
    let data = scc_engine::checkpoint::capture(root).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string(&data)?);
    } else {
        println!("checkpoint saved to {}", crate::checkpoint_path(root).display());
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-checkpoint-load work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_checkpoint_load(root: &Path, inject: bool) -> crate::Result<()> {
    if let Some(content) = scc_engine::checkpoint::load(root).map_err(engine_err)? {
        print!("{content}");
    } else if !inject {
        println!("no checkpoint found");
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-watch work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_watch(root: &Path) -> crate::Result<()> {
    crate::httpd::watch_loop(root)
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-serve work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_serve(root: &Path) -> crate::Result<()> {
    crate::httpd::serve(root)
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-mcp work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_mcp(root: &Path) -> crate::Result<()> {
    crate::mcp::serve_stdio(root)
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-setup-claude work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_setup_claude(root: &Path) -> crate::Result<()> {
    crate::plugin::install(root)
}

// Setup targets for harness auto-detection. Hermes is deliberately absent:
// it installs into a home directory outside the repo (different trust
// domain), so it stays an explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub enum SetupHarness {
    Claude,
    Codex,
    Opencode,
    Omp,
    Pi,
}

// Pure detection: a harness counts as present when its binary is on PATH
// or its home/config dir exists (or, for project-local OMP/Pi, the repo
// already carries the harness dir). `path_dirs` is PATH split already so
// tests never touch the process environment.
// trace:exempt reason=internal-detail
pub fn detect_harnesses(home: &Path, path_dirs: &[PathBuf], root: &Path) -> Vec<SetupHarness> {
    let on_path = |bin: &str| {
        path_dirs
            .iter()
            .any(|d| d.join(bin).is_file() || d.join(format!("{bin}.exe")).is_file())
    };
    let mut out = Vec::new();
    if on_path("claude") || home.join(".claude").is_dir() {
        out.push(SetupHarness::Claude);
    }
    if on_path("codex") || home.join(".codex").is_dir() {
        out.push(SetupHarness::Codex);
    }
    if on_path("opencode") || home.join(".config").join("opencode").is_dir() {
        out.push(SetupHarness::Opencode);
    }
    if on_path("omp") || root.join(".omp").is_dir() {
        out.push(SetupHarness::Omp);
    }
    if on_path("pi") || root.join(".pi").is_dir() || home.join(".pi").is_dir() {
        out.push(SetupHarness::Pi);
    }
    out
}

// trace:exempt reason=internal-detail
fn install_harness(root: &Path, h: SetupHarness) -> crate::Result<()> {
    match h {
        SetupHarness::Claude => cmd_setup_claude(root),
        SetupHarness::Codex => crate::compress::cmd_setup_codex(root),
        SetupHarness::Opencode => crate::compress::cmd_setup_opencode(root),
        SetupHarness::Omp => crate::plugin_omp::cmd_setup_omp(root),
        SetupHarness::Pi => crate::plugin_omp::cmd_setup_pi(root),
    }
}

/// `scc setup` (no subcommand): install for every detected harness and
/// print one summary, including the manual steps setup cannot perform
/// (Codex hooks live in user scope). `scc setup all` skips detection and
/// installs for every harness unconditionally.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-setup-detected work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_setup_detected(root: &Path, all: bool) -> crate::Result<()> {
    let targets: Vec<SetupHarness> = if all {
        vec![
            SetupHarness::Claude,
            SetupHarness::Codex,
            SetupHarness::Opencode,
            SetupHarness::Omp,
            SetupHarness::Pi,
        ]
    } else {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        detect_harnesses(&home, &path_dirs, root)
    };
    if targets.is_empty() {
        println!("No supported harness detected (looked for claude/codex/opencode/omp/pi");
        println!("binaries, ~/.claude, ~/.codex, ~/.config/opencode, ~/.pi, and .omp//.pi/ dirs).");
        println!("Run `scc setup all` to install for every harness, or `scc setup <harness>`.");
        return Ok(());
    }
    for h in &targets {
        println!("=== {:?} ===", h);
        install_harness(root, *h)?;
        println!();
    }
    println!("Installed for: {}", targets.iter().map(|h| format!("{h:?}")).collect::<Vec<_>>().join(", "));
    if targets.contains(&SetupHarness::Codex) {
        println!("Remaining manual step: add the printed entry to ~/.codex/hooks.json (user scope).");
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-ingest-runtime work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_ingest_runtime(root: &Path, body: &str) -> crate::Result<()> {
    scc_engine::state::ingest_runtime(root, body).map_err(engine_err)?;
    println!("accepted");
    Ok(())
}

/// Declared capability scope per adapter (docs/SECURITY.md §6: the doc
/// defines the manifest dimensions — FS scope, network, subprocess,
/// credentials — but has no per-adapter table, so the assignments are
/// pinned inline here). Importers read repo-local files only; Context7 runs
/// an MCP server over stdio via npx (subprocess) that makes network calls.
// trace:v1 id=impl.crates-scc-cli-src-commands.adapter-scope work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn adapter_scope(name: &str) -> &'static str {
    match name {
        "context7" => "network+subprocess(npx)",
        "serena" | "beads" | "cbm" | "hindsight" | "scip" | "gitnexus" | "narsil" => "filesystem",
        _ => "filesystem",
    }
}

/// Compute the configured-adapter scope listing from THE Integration
/// Registry: every row whose config key is enabled (or needs no config for
/// on-demand import), in fixed registry order. `narsil` never appears: it is
/// a `ccg` alias, resolved by `resolve_integration`. Both `scc adapters`
/// views read one registry, so they describe one universe.
// trace:v1 id=impl.crates-scc-cli-src-commands.configured-adapters work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
fn configured_adapters(root: &Path) -> crate::Result<Vec<(String, &'static str)>> {
    use scc_indexer::adapters::IntegrationCategory;
    let config = load_config(root)?;
    let enabled = |key: Option<&str>| -> bool {
        match key {
            None => true,
            Some("serena") => config.integrations.serena,
            Some("beads") => config.integrations.beads,
            Some("hindsight") => config.integrations.hindsight,
            Some("gitnexus") => config.integrations.gitnexus,
            Some("context7_command") => !config.integrations.context7_command.is_empty(),
            // Unknown keys fail closed: a new config flag must be wired
            // here explicitly, never silently treated as enabled.
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

/// `scc adapters` — list enabled adapters with their declared capability
/// scope (security audit; docs/SECURITY.md §6). `--json` dumps the full
/// capability manifests instead.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-adapters work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_adapters(root: &Path, json: bool) -> crate::Result<()> {
    let manifests = scc_indexer::adapters::adapter_manifests();
    if json {
        println!("{}", serde_json::to_string_pretty(&manifests)?);
        return Ok(());
    }
    for (name, scope) in configured_adapters(root)? {
        println!("adapter: {name}  scope: {scope}");
    }
    Ok(())
}

/// `scc doctor` — integration health from THE Integration Registry
/// (`scc_indexer::adapters::integration_registry`), offline, read-only and
/// fast. Default never touches the network and never spawns a subprocess:
/// every row is answered from local state only (config flags, binary on
/// PATH, files present, evidence already in the graph). `--deep` may start
/// LOCAL subprocesses (pyright/tsserver handshakes); `--network` may test
/// remote endpoints (Context7). `--strict` turns warnings into a nonzero
/// exit; `--json` emits the machine-readable report.
///
/// Honesty rule: a row reports IMPLEMENTED / CONFIGURED / REACHABLE /
/// CONTRIBUTING separately. A config boolean never implies contribution —
/// contribution means facts of that integration's evidence kind are actually
/// in the graph (or a handshake succeeded under --deep/--network).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-doctor work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_doctor(root: &Path, json: bool, deep: bool, network: bool, strict: bool) -> crate::Result<bool> {
    let store = open_store(root)?;
    let config = load_config(root)?;
    let rep = scc_engine::integrations::doctor_report(&store, &config, root, deep, network).map_err(engine_err)?;
    let rows = rep.integrations;
    let agent_rows = rep.agents;
    let mut warnings = rep.warnings;
    let (scc_version, rev, stale_len, dangling, stats) =
        (rep.scc_version, rep.revision, rep.stale_files, rep.dangling_edges, rep.stats);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "scc": scc_version,
                "revision": rev,
                "stale_files": stale_len,
                "dangling_edges": dangling,
                "stats": stats,
                "integrations": rows,
                "agents": agent_rows.iter().map(|a| serde_json::json!({"id": a.id, "present": a.present, "detail": a.detail})).collect::<Vec<_>>(),
                "warnings": warnings,
            }))?
        );
    } else {
        println!("SCC Doctor");
        println!("==========");
        println!();
        println!("CORE");
        println!("  scc        {scc_version}");
        println!("  revision   {rev}");
        println!("  stale      {} file(s)", stale_len);
        println!("  dangling   {dangling} edge(s) (see `scc check-invariants`)");
        println!("  entities   {}", stats.get("entities").copied().unwrap_or(0));
        println!();
        println!("INTEGRATIONS");
        for r in &rows {
            let mark = if r.contributing {
                "✓"
            } else if r.configured {
                "!"
            } else {
                "-"
            };
            let reach = match r.reachable {
                Some(true) => "reachable; ",
                Some(false) => "UNREACHABLE; ",
                None => "",
            };
            println!("  {mark} {:<12} [{:<18}] {}{}", r.id, r.category, reach, r.detail);
        }
        println!();
        println!("AGENTS");
        for a in &agent_rows {
            println!("  {} {:<12} {}", if a.present { "✓" } else { "-" }, a.id, a.detail);
        }
        println!();
        if warnings == 0 {
            println!("SUMMARY  PASS ({} integrations contributing evidence)", rows.iter().filter(|r| r.contributing).count());
        } else {
            println!("SUMMARY  {warnings} warning(s); run `scc doctor --deep` / `--network` for handshake detail");
        }
    }
    if dangling > 0 {
        warnings += 1;
    }
    // `scc check-invariants` remains the CI gate for graph integrity;
    // doctor reports health (warnings) by default and fails only on
    // --strict, so informational runs stay exit-0.
    Ok(!(strict && warnings > 0))
}

/// `scc lessons add <text>` — append one durable lesson to
/// `<root>/.scc/lessons.jsonl` (the Hindsight memory bank). The bank is
/// ingested into the System IR with `scc import hindsight .scc/lessons.jsonl`.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-lessons-add work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_lessons_add(root: &Path, text: &str) -> crate::Result<()> {
    let dir = scc_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("lessons.jsonl");
    let n = std::fs::read_to_string(&path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let id = format!("lesson-{}", n + 1);
    let record = serde_json::json!({
        "id": id,
        "text": text,
        "created_at": scc_core::now_rfc3339(),
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| crate::CliError::Other(format!("lessons: {e}")))?;
    writeln!(f, "{record}").map_err(|e| crate::CliError::Other(format!("lessons: {e}")))?;
    println!(
        "appended {id} to {} (ingest with `scc import hindsight .scc/lessons.jsonl`)",
        path.display()
    );
    Ok(())
}

/// `scc lessons` — list stored lessons from the System IR (most important
/// first, same ordering as the context-pack enrichment).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-lessons-list work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_lessons_list(root: &Path, limit: usize) -> crate::Result<()> {
    let lessons = scc_engine::state::lessons_list(root, limit).map_err(engine_err)?;
    if lessons.is_empty() {
        println!("no lessons in the store — run `scc lessons add \"...\"` then `scc import hindsight .scc/lessons.jsonl`");
        return Ok(());
    }
    for (content, tags) in lessons {
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("- {content}{tag_str}");
    }
    Ok(())
}

/// `scc beads` — list active (in-progress) tasks from `.beads/issues.jsonl`
/// (task state, not system facts).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-beads work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_beads(root: &Path) -> crate::Result<()> {
    let active = scc_engine::state::beads(root, 20).map_err(engine_err)?;
    if active.is_empty() {
        println!("no active beads tasks (checked .beads/issues.jsonl)");
        return Ok(());
    }
    println!("active beads tasks:");
    for t in active {
        println!("- {t}");
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-import work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_import(root: &Path, format: &str, file: &str) -> crate::Result<()> {
    let report = scc_engine::state::import_evidence(root, format, file).map_err(engine_err)?;
    // P0: imported evidence changes system truth — the engine already bumped
    // the evidence epoch and recompiled the derived layer.
    println!(
        "imported {} symbols, {} calls, {} imports ({} errors); evidence epoch bumped, derived layer recompiled",
        report.symbols, report.calls, report.imports, report.errors
    );
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-runtime-status work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_runtime_status(root: &Path, json: bool) -> crate::Result<()> {
    let edges = scc_engine::state::runtime_edges(root).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&edges)?);
        return Ok(());
    }
    if edges.is_empty() {
        println!("no runtime observations ingested");
    }
    let total: u64 = edges.iter().map(|e| e.count).sum();
    let errs: u64 = edges.iter().map(|e| e.errors).sum();
    println!("{} observed edge(s), {} observations, {} error(s)", edges.len(), total, errs);
    for e in &edges {
        println!(
            "- {} → {} ×{} (avg {:.1} ms, {} err, last {})",
            e.source, e.target, e.count, e.latency_ms, e.errors, e.last_observed
        );
    }
    Ok(())
}

// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-runtime-reconcile work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn cmd_runtime_reconcile(root: &Path, json: bool) -> crate::Result<()> {
    let rec = scc_engine::state::reconcile(root).map_err(engine_err)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
        return Ok(());
    }
    println!("static-vs-observed reconciliation");
    println!("  matched:             {}", rec.matched.len());
    println!("  observed not static: {}", rec.observed_not_static.len());
    println!("  static not observed: {}", rec.static_not_observed.len());
    for e in &rec.matched {
        println!("  [matched] {e}");
    }
    for e in &rec.observed_not_static {
        println!("  [runtime-only] {e}");
    }
    for e in &rec.static_not_observed {
        println!("  [static-only] {e}");
    }
    Ok(())
}

/// `scc diagram` — SCC-native architecture diagram (SPEC-SCC-VIEWER §1).
/// L1 nodes plus capped architectural edges plus flow subgraphs, as Mermaid
/// (default) or dependency-free SVG. Deterministic: same index, same bytes.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-diagram work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn cmd_diagram(root: &Path, format: &str, out: Option<&str>) -> crate::Result<()> {
    let store = open_store(root)?;
    if store.snapshot_status()?.is_none() {
        return Err(crate::CliError::Other("not indexed yet — run `scc index`".into()));
    }
    let model = crate::viewer::build_diagram_model(&store)?;
    let text = match format {
        "mermaid" => crate::viewer::render_mermaid(&model),
        "svg" => crate::viewer::render_svg(&model),
        other => {
            return Err(crate::CliError::Other(format!(
                "unknown diagram format '{other}' (use mermaid|svg)"
            )))
        }
    };
    if let Some(path) = out {
        std::fs::write(path, &text)?;
        println!(
            "diagram: {} nodes, {} edges, {} flows -> {path}",
            model.nodes.len(),
            model.edges.len(),
            model.flows.len()
        );
    } else {
        print!("{text}");
    }
    Ok(())
}

/// `scc snap` — Snapcompact-style bitmap export (SPEC-SCC-VIEWER §3).
/// Default OFF: requires `--out` text plus `--png` (or config
/// `context.snap_enabled`) to render; otherwise prints the map text and the
/// pinned Pillow recipe so the operator sees exactly what would ship.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-snap work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn cmd_snap(
    root: &Path,
    out: Option<&str>,
    max_chars: usize,
    png: Option<&str>,
    color: bool,
) -> crate::Result<()> {
    let store = open_store(root)?;
    if store.snapshot_status()?.is_none() {
        return Err(crate::CliError::Other("not indexed yet — run `scc index`".into()));
    }
    let config = load_config(root)?;
    let map = crate::viewer::map_text(&store, max_chars)?;
    let rows = map.lines().count();
    let (text_tokens, image_tokens) = crate::viewer::token_estimate(map.len(), rows);
    let render = png.is_some() || config.context.snap_enabled;
    if let Some(path) = out {
        std::fs::write(path, &map)?;
    } else {
        print!("{map}");
        if !map.ends_with('\n') {
            println!();
        }
    }
    if color {
        eprintln!("note: --color is accepted for recipe parity; the pinned recipe renders monochrome unless edited");
    }
    println!("snap: {rows} rows, {} chars — text ~{text_tokens} tokens vs PNG ~{image_tokens} image tokens", map.len());
    if render {
        let dest = png.unwrap_or("scc-snap.png");
        render_snap_png(&map, dest)?;
        println!("snap png -> {dest}");
    } else {
        println!("render OFF by default (set context.snap_enabled=true or pass --png <file>); recipe:");
        println!("{}", crate::viewer::snap_recipe());
    }
    Ok(())
}

/// Render the map text to PNG via the pinned Pillow recipe. Shells out to
/// `python3` only when rendering was explicitly requested; falls back to
/// printing the recipe path when Pillow is missing.
// trace:v1 id=impl.crates-scc-cli-src-commands.render-snap-png work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn render_snap_png(map: &str, dest: &str) -> crate::Result<()> {
    // Fail fast with an actionable error when Pillow is missing (CI has no
    // PIL): the operator installs Pillow or uses the printed recipe.
    let probe = std::process::Command::new("python3")
        .arg("-c")
        .arg("import PIL")
        .output()
        .map_err(|e| crate::CliError::Other(format!("snap: cannot run python3 ({e})")))?;
    if !probe.status.success() {
        return Err(crate::CliError::Other(
            "snap: python3 has no Pillow (pip install pillow); map text written, recipe printed above".into(),
        ));
    }
    let dir = tempfile::TempDir::new()?;
    let map_path = dir.path().join("map.txt");
    let recipe_path = dir.path().join("snap.py");
    std::fs::write(&map_path, map)?;
    std::fs::write(&recipe_path, crate::viewer::snap_recipe())?;
    let out = std::process::Command::new("python3")
        .arg(&recipe_path)
        .arg(&map_path)
        .arg(dest)
        .output()
        .map_err(|e| crate::CliError::Other(format!("snap: cannot run python3 ({e}); recipe printed above")))?;
    if !out.status.success() {
        return Err(crate::CliError::Other(format!(
            "snap: Pillow render failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// `scc view` — local web viewer (SPEC-SCC-VIEWER §2). Serves the loopback
/// viewer routes on an ephemeral port (or `--port`), then opens the
/// browser unless `--no-open`.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-view work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn cmd_view(root: &Path, port: Option<u16>, no_open: bool) -> crate::Result<()> {
    let store = open_store(root)?;
    if store.snapshot_status()?.is_none() {
        return Err(crate::CliError::Other("not indexed yet — run `scc index`".into()));
    }
    let addr = match port {
        Some(p) => format!("127.0.0.1:{p}"),
        None => "127.0.0.1:0".to_string(),
    };
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| crate::CliError::Other(format!("cannot bind {addr}: {e}")))?;
    let bound = server.server_addr();
    let url = format!("http://{bound}/");
    println!("scc view: {url} (root {})", root.display());
    if !no_open {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
    for request in server.incoming_requests() {
        let url_path = request.url().to_string();
        let method = request.method().clone();
        if method == tiny_http::Method::Get && crate::viewer::is_viewer_path(&url_path) {
            let store = match open_store(root) {
                Ok(s) => s,
                Err(e) => {
                    let _ = request.respond(
                        tiny_http::Response::from_string(format!("store error: {e}"))
                            .with_status_code(500),
                    );
                    continue;
                }
            };
            let (status, body) = crate::viewer::serve_viewer(&store, &url_path);
            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                        .unwrap(),
                );
            let _ = request.respond(response);
            continue;
        }
        // Viewer-only server: the daemon (`scc serve`) owns /v1/*.
        // Anything else is a 404 so typos fail loudly, not silently.
        let _ = request.respond(
            tiny_http::Response::from_string("no such viewer route (scc serve owns /v1/*)")
                .with_status_code(404),
        );
    }
    Ok(())
}

/// `scc rpc --stdio`: structured JSON-RPC (SDK subprocess mode).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-rpc work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_rpc(root: &Path, _stdio: bool) -> crate::Result<()> {
    scc_engine::rpc::serve_stdio(root).map_err(engine_err)
}

/// `scc operations [--describe ID]`: introspection over the registry.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-operations work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_operations(describe: Option<&str>) -> crate::Result<()> {
    if let Some(id) = describe {
        match scc_engine::ops::describe(id) {
            Some(d) => println!("{}", serde_json::to_string_pretty(&d)?),
            None => println!("unknown operation '{id}' (see `scc operations`)"),
        }
        return Ok(());
    }
    println!("Operation                 Mutation  Stream  Description");
    println!("----------------------------------------------------------------");
    for d in scc_engine::ops::OPERATIONS {
        println!("{:<26} {:<9} {:<7} {}", d.id, format!("{:?}", d.mutation), d.streaming, d.description);
    }
    Ok(())
}

/// `scc plugin list`: enabled plugins with lock entries (engine owns the set).
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-list work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_list(root: &Path) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "plugins.list", serde_json::json!({})).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// `scc plugin describe <id>`.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-describe work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_describe(root: &Path, id: &str) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "plugins.describe", serde_json::json!({"id": id})).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// `scc plugin doctor`.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-doctor work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_doctor(root: &Path) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "plugins.doctor", serde_json::json!({})).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// `scc plugin lock`: write .scc/plugins.lock from the live set.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-lock work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_lock(root: &Path) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "plugins.lock", serde_json::json!({})).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// `scc plugin check`: verify live plugins against .scc/plugins.lock.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-check work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_check(root: &Path) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "plugins.check", serde_json::json!({})).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    if out.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(crate::CliError::Other(format!(
            "plugin lock drift: {}",
            out.get("drift").map(|v| v.to_string()).unwrap_or_default()
        )));
    }
    Ok(())
}

/// `scc plugin invoke <operation> [json-input]`.
// trace:v1 id=impl.crates-scc-cli-src-commands.cmd-plugin-invoke work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_plugin_invoke(root: &Path, operation: &str, input: &str) -> crate::Result<()> {
    let input: serde_json::Value = serde_json::from_str(input).unwrap_or(serde_json::json!({}));
    let out = scc_engine::invoke(root, operation, input).map_err(engine_err)?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    // trace:exempt reason=unit-test
    fn ensure_scc_ignored_keeps_intent_committable() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::ensure_scc_ignored(root);
        let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gi.contains(".scc/*"), "{gi}");
        assert!(gi.contains("!.scc/intent.yaml"), "{gi}");
        crate::ensure_scc_ignored(root);
        let gi2 = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gi, gi2, "idempotent");
        std::fs::write(root.join(".gitignore"), "target/\n.scc/\n").unwrap();
        crate::ensure_scc_ignored(root);
        let gi3 = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gi3.contains("target/"), "{gi3}");
        assert!(!gi3.lines().any(|l| l.trim() == ".scc/"), "{gi3}");
        assert!(gi3.contains("!.scc/intent.yaml"), "{gi3}");
    }

    #[test]
    // trace:exempt reason=unit-test
    fn detect_harnesses_finds_bins_dirs_and_project_dirs() {
        let home = tempfile::TempDir::new().unwrap();
        let bindir = home.path().join("bin");
        std::fs::create_dir_all(&bindir).unwrap();
        std::fs::write(bindir.join("codex"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(home.path().join(".config").join("opencode")).unwrap();
        let root = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(root.path().join(".omp")).unwrap();
        let found = detect_harnesses(
            home.path(),
            &[bindir],
            root.path(),
        );
        assert!(found.contains(&SetupHarness::Codex), "codex via PATH: {found:?}");
        assert!(found.contains(&SetupHarness::Opencode), "opencode via config dir: {found:?}");
        assert!(found.contains(&SetupHarness::Omp), "omp via project dir: {found:?}");
        assert!(!found.contains(&SetupHarness::Claude), "no claude present: {found:?}");
        assert!(!found.contains(&SetupHarness::Pi), "no pi present: {found:?}");
    }

    #[test]
    // trace:exempt reason=unit-test
    fn detect_harnesses_empty_when_nothing_present() {
        let home = tempfile::TempDir::new().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let found = detect_harnesses(home.path(), &[], root.path());
        assert!(found.is_empty(), "{found:?}");
    }

    use super::*;


    #[test]
// trace:v1 id=impl.crates-scc-cli-src-commands.lessons-add-appends-jsonl-lines work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn lessons_add_appends_jsonl_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        cmd_lessons_add(&root, "first lesson").unwrap();
        cmd_lessons_add(&root, "second lesson").unwrap();
        let path = root.join(".scc/lessons.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "a second add must append, not overwrite");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["id"], "lesson-1");
        assert_eq!(first["text"], "first lesson");
        assert!(first["created_at"].is_string());
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["id"], "lesson-2");
        assert_eq!(second["text"], "second lesson");
        // the shape must be ingestable by the hindsight adapter
        let lesson: scc_indexer::adapters::hindsight::Lesson =
            serde_json::from_str(lines[0]).unwrap();
        assert_eq!(lesson.body(), "first lesson");
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-commands.adapters-lists-configured-scope work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn adapters_lists_configured_scope() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".scc")).unwrap();
        std::fs::write(
            root.join(".scc/config.yaml"),
            "schema: 1\nintegrations:\n  beads: true\n  context7_command: \"npx -y @upstash/context7-mcp\"\n",
        )
        .unwrap();
        let listing = configured_adapters(&root).unwrap();
        assert!(
            listing.contains(&("beads".to_string(), "filesystem")),
            "{listing:?}"
        );
        assert!(
            listing.contains(&("context7".to_string(), "network+subprocess(npx)")),
            "{listing:?}"
        );
        assert!(
            !listing.iter().any(|(n, _)| n == "hindsight"),
            "disabled integrations must not be listed: {listing:?}"
        );
        // scip/cbm are always-available on-demand importers
        assert!(listing.iter().any(|(n, _)| n == "scip"), "{listing:?}");
        assert!(listing.iter().any(|(n, _)| n == "cbm"), "{listing:?}");
        // One universe: every listed adapter resolves in the registry, and
        // the registry's evidence importers appear (tracelayer was missing).
        for (n, _) in &listing {
            assert!(
                scc_indexer::adapters::resolve_integration(n).is_some(),
                "listed adapter {n} must resolve in the registry: {listing:?}"
            );
        }
        assert!(listing.iter().any(|(n, _)| n == "tracelayer"), "{listing:?}");
        // narsil is a ccg alias, never a listed adapter.
        assert!(!listing.iter().any(|(n, _)| n == "narsil"), "{listing:?}");
        assert!(listing.iter().any(|(n, _)| n == "ccg"), "{listing:?}");
        // cmd_adapters renders the exact expected line format
        cmd_adapters(&root, false).unwrap();
        // doctor is offline, read-only, exit-0 by default on this fixture.
        assert!(cmd_doctor(&root, false, false, false, false).unwrap(), "doctor must pass clean");
        assert!(cmd_doctor(&root, true, false, false, false).unwrap(), "doctor --json must pass clean");
    }
}

