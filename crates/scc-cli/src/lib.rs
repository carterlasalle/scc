//! System Context Compiler CLI, daemon, MCP server, and Claude Code plugin.

pub mod agents_md;
pub mod bench;
pub mod benchagent;
pub mod benchatlas;
pub mod benchctx;
pub mod benchloop;
pub mod benchres;
pub mod benchret;
pub mod checkpoint;
pub mod commands;
pub mod compress;
pub mod embed_cli;
pub mod httpd;
pub mod mcp;
pub mod plugin;
pub mod plugin_hermes;
pub mod plugin_omp;
pub mod resolve;

use scc_context::ContextCompiler;
use scc_graph::RealityGraph;
use scc_indexer::Config;
use scc_store::Store;
use std::path::{Path, PathBuf};

pub const SCC_DIR: &str = ".scc";
pub const DB_FILE: &str = "scc.db";
pub const CONFIG_FILE: &str = "config.yaml";
pub const CHECKPOINT_FILE: &str = "checkpoint.json";

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
    #[error("index: {0}")]
    Index(#[from] scc_indexer::IndexError),
    #[error("graph: {0}")]
    Graph(#[from] scc_graph::GraphError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(#[from] scc_indexer::config::ConfigError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CliError>;

/// SCC state directory: the repo's `.scc/` by default; `SCC_STATE_DIR`
/// relocates writable state (database, checkpoint) so the repository itself
/// can be mounted read-only (docs/DEPLOYMENT_AND_INFRA.md §3: read-only repo
/// + writable SCC data volume).
pub fn state_dir(root: &Path) -> PathBuf {
    match std::env::var("SCC_STATE_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => scc_dir(root),
    }
}

pub fn scc_dir(root: &Path) -> PathBuf {
    root.join(SCC_DIR)
}

pub fn db_path(root: &Path) -> PathBuf {
    state_dir(root).join(DB_FILE)
}

/// Config stays in the repo (read-only is fine): it is repository intent,
/// not SCC state.
pub fn config_path(root: &Path) -> PathBuf {
    scc_dir(root).join(CONFIG_FILE)
}

pub fn checkpoint_path(root: &Path) -> PathBuf {
    state_dir(root).join(CHECKPOINT_FILE)
}

/// Locate the repository root: walk up from cwd looking for `.git` or an
/// existing `.scc` dir; otherwise use cwd.
pub fn find_root(start: &Path) -> PathBuf {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join(".git").exists() || d.join(SCC_DIR).exists() {
            return d;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    start.to_path_buf()
}

pub fn load_config(root: &Path) -> Result<Config> {
    let p = config_path(root);
    if p.exists() {
        Ok(Config::load(&p)?)
    } else {
        Ok(Config::default())
    }
}

pub fn open_store(root: &Path) -> Result<Store> {
    let dir = state_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(Store::open(&db_path(root), root)?)
}

/// Ensure `.scc/` is gitignored so the index cache never pollutes the
/// repo's own git status or gets committed — except `.scc/intent.yaml`,
/// which is committable repository intent, not cache. A bare `.scc/`
/// pattern would make git (and our own gitignore-respecting walker) prune
/// the whole directory including intent, silently dropping declared
/// components and flows. Idempotent, never touches other lines.
// trace:v1 id=impl.crates-scc-cli-src-lib.ensure-scc-ignored work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn ensure_scc_ignored(root: &Path) {
    // A bare `.scc/` line excludes the directory itself, which git does
    // not let negations re-enter — migrate it to the pair so intent.yaml
    // stays committable while the cache stays out.
    const WANT: [&str; 2] = [".scc/*", "!.scc/intent.yaml"];
    let gi = root.join(".gitignore");
    let content = std::fs::read_to_string(&gi).unwrap_or_default();
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut changed = false;
    lines.retain(|l| {
        let bare = l.trim() == ".scc/" || l.trim() == ".scc";
        if bare {
            changed = true;
        }
        !bare
    });
    for line in WANT {
        if !lines.iter().any(|l| l.trim() == line) {
            lines.push(line.to_string());
            changed = true;
        }
    }
    if changed {
        let mut out = lines.join("\n");
        out.push('\n');
        let _ = std::fs::write(&gi, out);
    }
}

/// True when an indexing failure is store corruption surfacing anywhere in
/// the error chain (open, mid-index read, recompile) — not just at open.
/// Corruption can hide behind a valid header and detonate on first touch of
/// a bad page, so the write path retries from quarantine on any of these.
// trace:exempt reason=internal-detail
pub fn is_store_corruption(e: &CliError) -> bool {
    match e {
        CliError::Store(s) => Store::is_corruption(s),
        CliError::Index(scc_indexer::IndexError::Store(s)) => Store::is_corruption(s),
        CliError::Graph(scc_graph::GraphError::Store(s)) => Store::is_corruption(s),
        _ => false,
    }
}

/// Run an index write; on store corruption anywhere in the attempt,
/// quarantine the database and retry exactly once from scratch.
// trace:v1 id=impl.crates-scc-cli-src-lib.resilient-index work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn resilient_index<T>(
    root: &Path,
    mut attempt: impl FnMut() -> Result<T>,
) -> Result<T> {
    match attempt() {
        Ok(v) => Ok(v),
        Err(e) if is_store_corruption(&e) => {
            let q = Store::quarantine_db(&db_path(root))?;
            report_quarantine(&Some(q));
            attempt()
        }
        Err(e) => Err(e),
    }
}

// trace:v1 id=impl.crates-scc-cli-src-lib.open-store-recovering work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn open_store_recovering(root: &Path) -> Result<(Store, Option<std::path::PathBuf>)> {
    let dir = state_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(Store::open_recovering(&db_path(root), root)?)
}

// trace:exempt reason=internal-detail
pub fn report_quarantine(quarantined: &Option<std::path::PathBuf>) {
    if let Some(q) = quarantined {
        eprintln!(
            "warning: existing index was corrupt (malformed database); quarantined to {} and rebuilding from scratch.",
            q.display()
        );
    }
}

pub fn recompile(store: &Store) -> Result<scc_graph::RecompileReport> {
    Ok(scc_graph::recompile(store)?)
}

/// Compute repository-relative paths whose indexed snapshot no longer matches
/// the working tree: modified, deleted, AND added files, from ONE
/// authoritative scan diffed against the indexed inventory — a newly
/// created relevant file must make the model non-current, and indexed
/// files are never re-read when the scan already hashed them (one
/// read+hash per file per freshness check, not two). Same scan the
/// indexer uses, so both sides share one notion of "repository file"
/// (git-ignored and configured-ignored paths excluded).
///
/// Scaling note: the authoritative scan still reads every candidate file
/// (correctness first — no mtime cache exists yet, so content hashing is
/// the only proof of sameness). The scan walk itself is the periodic
/// reconciliation; a watcher dirty-set + metadata fast path stays
/// deferred until a daemon owns it.
// trace:v1 id=impl.crates-scc-cli-src-lib.stale-paths work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub fn stale_paths(store: &Store) -> Result<Vec<String>> {
    let config = load_config(&store.root)?;
    let scanned = scc_indexer::scan::scan_repo(&store.root, &config.index).map_err(scc_indexer::IndexError::from)?;
    let mut fresh: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for f in &scanned {
        fresh.insert(f.path.as_str(), f.hash.as_str());
    }
    let mut out = Vec::new();
    let mut indexed = std::collections::HashSet::new();
    for (path, hash, _lang, _kind, _size) in store.all_files()? {
        indexed.insert(path.clone());
        match fresh.get(path.as_str()) {
            Some(current) if *current == hash.as_str() => {} // fresh — no re-read
            Some(_) => out.push(path),   // modified
            None => out.push(path),      // deleted (or newly ignored)
        }
    }
    for f in scanned {
        if !indexed.contains(&f.path) {
            out.push(f.path); // added since indexing
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

// trace:exempt reason=existing
pub struct Compiler<'a> {
    pub store: &'a Store,
    pub graph: RealityGraph,
    pub settings: scc_context::ContextSettings,
    pub stale: Vec<String>,
}

/// Build a ready compiler with freshness state.
// trace:v1 id=impl.crates-scc-cli-src-lib.compiler work=WORK-SCC-001 satisfies=REQ-SCC-API
pub fn compiler<'a>(
    store: &'a Store,
    config: &Config,
    stale: Vec<String>,
) -> Result<Compiler<'a>> {
    let graph = RealityGraph::load(store)?;
    let settings = scc_context::ContextSettings {
        startup_tokens: config.context.startup_tokens,
        task_tokens: config.context.task_tokens,
        atlas_tokens: config.context.atlas_tokens,
        detail_tokens: config.context.detail_tokens,
        include_low_confidence_inference: config.context.include_low_confidence_inference,
        rank_salt: format!(
            "{}:{}:{}",
            config.inference.enabled,
            config.inference.embedding_model,
            config.inference.rerank_model
        ),
        pack_allocator: scc_context::PackAllocator::AdaptivePriority,
    };
    Ok(Compiler {
        store,
        graph,
        settings,
        stale,
    })
}

impl Compiler<'_> {
    /// Construct a ContextCompiler borrowing this compiler's graph.
    pub fn ctx(&self) -> ContextCompiler<'_> {
        ContextCompiler::new(
            self.store,
            &self.graph,
            self.settings.clone(),
            self.stale.clone(),
        )
    }
}

// trace:exempt reason=internal-detail
pub fn index_and_recompile(root: &Path, config: &Config) -> Result<scc_indexer::IndexReport> {
    ensure_scc_ignored(root);
    resilient_index(root, || {
        let (store, quarantined) = open_store_recovering(root)?;
        report_quarantine(&quarantined);
    let indexer = scc_indexer::Indexer::new(store, config.clone());
    let report = indexer.index()?;
    let store = open_store(root)?;
    // Wave 4 §24 lazy semantic enrichment: when auto_resolve is on, run the
    // language-aware backends (pyright/tsserver) before the derived layer
    // compiles, so flows/atlas see RESOLVED edges.
    if config.index.auto_resolve {
        let _ = scc_indexer::resolver::resolve_repository(
            &store,
            root,
            scc_indexer::resolver::MAX_CALL_SITES,
        );
    }
    recompile(&store)?;
    // Revision AFTER recompile (never inside the indexer): history must
    // include derived facts (components, boundaries, flows). Recording
    // before recompile leaves history one recompile behind — V2 content
    // dedup exposed this ordering bug.
        let _ = store.record_current_revision_with_config(
            &scc_indexer::semantic_config_hash(config),
        )?;
        Ok(report)
    })
}

/// Run semantic resolution on demand (`--resolve`), then recompile the
/// derived layer so graphs/flows/atlas reflect the promoted edges.
pub fn resolve_and_recompile(root: &Path) -> Result<scc_indexer::resolver::ResolveReport> {
    let store = open_store(root)?;
    let report = scc_indexer::resolver::resolve_repository(
        &store,
        root,
        scc_indexer::resolver::MAX_CALL_SITES,
    )
    .map_err(CliError::Other)?;
    recompile(&store)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// export (docs/DATA_STRATEGY.md §11)
// ---------------------------------------------------------------------------

pub fn export_ir(store: &Store) -> Result<scc_core::SystemIr> {
    let repository = store.repository();
    let snapshot = store
        .latest_snapshot()?
        .unwrap_or(scc_core::Snapshot {
            revision: "not-indexed".into(),
            branch: None,
            indexed_at: scc_core::now_rfc3339(),
        });
    let mut ir = scc_core::SystemIr::empty(repository, snapshot);
    // entities: everything except the derived component copies (they are
    // already stored as entities by replace_components — dedupe)
    let mut seen = std::collections::HashSet::new();
    for e in store.all_entities()? {
        if seen.insert(e.id.clone()) {
            ir.entities.push(e);
        }
    }
    ir.relationships = store.all_relationships()?;
    ir.flows = store.flows()?;
    ir.invariants = store.invariants()?;
    ir.evidence = store.all_evidence()?;
    Ok(ir)
}

/// JSONL export: one JSON object per line (repository, snapshot, then
/// entities/relationships/flows/invariants/evidence records).
pub fn export_jsonl(ir: &scc_core::SystemIr) -> Result<Vec<String>> {
    let mut out = Vec::new();
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "repository", "repository": ir.repository
    }))?);
    out.push(serde_json::to_string(&serde_json::json!({
        "type": "snapshot", "snapshot": ir.snapshot, "schema_version": ir.schema_version
    }))?);
    for e in &ir.entities {
        out.push(serde_json::to_string(&serde_json::json!({"type": "entity", "entity": e}))?);
    }
    for r in &ir.relationships {
        out.push(serde_json::to_string(&serde_json::json!({"type": "relationship", "relationship": r}))?);
    }
    for f in &ir.flows {
        out.push(serde_json::to_string(&serde_json::json!({"type": "flow", "flow": f}))?);
    }
    for i in &ir.invariants {
        out.push(serde_json::to_string(&serde_json::json!({"type": "invariant", "invariant": i}))?);
    }
    for e in &ir.evidence {
        out.push(serde_json::to_string(&serde_json::json!({"type": "evidence", "evidence": e}))?);
    }
    Ok(out)
}

/// Narsil-CCG-compatible layered export (docs §44): L0 manifest, L1
/// architecture, L2 symbols.
pub fn export_ccg(ir: &scc_core::SystemIr) -> Result<serde_json::Value> {
    let l1: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| {
            e.kind == kinds::COMPONENT
                || e.kind == kinds::SERVICE
                || e.kind == kinds::DATA_STORE
                || e.kind == kinds::DEPLOYMENT_UNIT
                || e.kind == kinds::EXTERNAL_API
        })
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.kind,
                "attributes": e.attributes,
            })
        })
        .collect();
    let l2: Vec<serde_json::Value> = ir
        .entities
        .iter()
        .filter(|e| e.kind == kinds::SYMBOL)
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "kind": e.attributes.get("kind").cloned().unwrap_or(serde_json::json!("symbol")),
                "file": e.attributes.get("file").cloned().unwrap_or_default(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema": "ccg",
        "producer": "scc",
        "repository": ir.repository,
        "snapshot": ir.snapshot,
        "layers": {
            "L0": {
                "manifest": {
                    "repository": ir.repository.name,
                    "revision": ir.snapshot.revision,
                    "entity_count": ir.entities.len(),
                    "relationship_count": ir.relationships.len(),
                }
            },
            "L1": { "architecture": l1 },
            "L2": { "symbols": l2 },
        }
    }))
}

pub fn flow_kind_str(k: &scc_core::FlowKind) -> &'static str {
    match k {
        scc_core::FlowKind::Architecture => "architecture",
        scc_core::FlowKind::Workflow => "workflow",
        scc_core::FlowKind::Sequence => "sequence",
        scc_core::FlowKind::Dataflow => "dataflow",
        scc_core::FlowKind::Lifecycle => "lifecycle",
    }
}

pub use scc_core::kinds;

/// Repo-relative path of a file under root, or None if it escapes.
pub fn relative_of(root: &Path, abs: &Path) -> Option<String> {
    let root_c = root.canonicalize().ok()?;
    let abs_c = abs.canonicalize().ok()?;
    let rel = abs_c.strip_prefix(&root_c).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.starts_with(".scc/") {
        return None;
    }
    Some(s)
}
