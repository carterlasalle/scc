//! Engine workspace: open/config/freshness — the preamble every
//! operation repeats (moved verbatim from `scc-cli` lib.rs so all
//! transports share one implementation).

use crate::context::SccContext;
use scc_context::ContextCompiler;
use scc_graph::RealityGraph;
use scc_indexer::Config;
use scc_store::Store;
use std::path::{Path, PathBuf};

// trace:exempt reason=internal-detail
pub const SCC_DIR: &str = ".scc";
// trace:exempt reason=internal-detail
pub const DB_FILE: &str = "scc.db";
// trace:exempt reason=internal-detail
pub const CONFIG_FILE: &str = "config.yaml";
// trace:exempt reason=internal-detail
pub const CHECKPOINT_FILE: &str = "checkpoint.json";

// trace:exempt reason=internal-detail
pub fn state_dir(root: &Path) -> PathBuf {
    match std::env::var("SCC_STATE_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => scc_dir(root),
    }
}

// trace:exempt reason=internal-detail
pub fn scc_dir(root: &Path) -> PathBuf {
    root.join(SCC_DIR)
}

// trace:exempt reason=internal-detail
pub fn db_path(root: &Path) -> PathBuf {
    state_dir(root).join(DB_FILE)
}

// trace:exempt reason=internal-detail
pub fn config_path(root: &Path) -> PathBuf {
    scc_dir(root).join(CONFIG_FILE)
}

// trace:exempt reason=internal-detail
pub fn checkpoint_path(root: &Path) -> PathBuf {
    state_dir(root).join(CHECKPOINT_FILE)
}

// trace:exempt reason=internal-detail
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

// trace:exempt reason=internal-detail
pub fn load_config(root: &Path) -> crate::Result<Config> {
    let p = config_path(root);
    if p.exists() {
        Ok(Config::load(&p)?)
    } else {
        Ok(Config::default())
    }
}

// trace:exempt reason=internal-detail
pub fn open_store(root: &Path) -> crate::Result<Store> {
    let dir = state_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(Store::open(&db_path(root), root)?)
}

// trace:exempt reason=internal-detail
pub fn open_store_recovering(root: &Path) -> crate::Result<(Store, Option<PathBuf>)> {
    let dir = state_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(Store::open_recovering(&db_path(root), root)?)
}

// trace:exempt reason=internal-detail
pub fn report_quarantine(quarantined: &Option<PathBuf>) {
    if let Some(q) = quarantined {
        eprintln!(
            "warning: existing index was corrupt (malformed database); quarantined to {} and rebuilding from scratch.",
            q.display()
        );
    }
}

// trace:exempt reason=internal-detail
pub fn is_store_corruption(e: &crate::EngineError) -> bool {
    match e {
        crate::EngineError::Store(s) => Store::is_corruption(s),
        crate::EngineError::Index(scc_indexer::IndexError::Store(s)) => Store::is_corruption(s),
        crate::EngineError::Graph(scc_graph::GraphError::Store(s)) => Store::is_corruption(s),
        _ => false,
    }
}

// trace:exempt reason=internal-detail
pub fn resilient_index<T>(
    root: &Path,
    mut attempt: impl FnMut() -> crate::Result<T>,
) -> crate::Result<T> {
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

// trace:exempt reason=internal-detail
pub fn ensure_scc_ignored(root: &Path) {
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

// trace:exempt reason=existing
pub struct Engine<'a> {
    pub store: &'a Store,
    pub graph: RealityGraph,
    pub settings: scc_context::ContextSettings,
    pub stale: Vec<String>,
}

// trace:exempt reason=internal-detail
pub use crate::plugins::cache_key_fragment;
// trace:exempt reason=internal-detail
pub fn open_engine<'a>(
    store: &'a Store,
    config: &Config,
    stale: Vec<String>,
) -> crate::Result<Engine<'a>> {
    let graph = RealityGraph::load(store)?;
    // Plugin lock folds into the salt: task/atlas pack caches key on
    // rank_salt, so a plugin install/upgrade/removal must change the key
    // (spec 27 — an unkeyed cache would serve pre-plugin packs as fresh).
    let plugin_salt = cache_key_fragment(&crate::plugins::active(&store.root, config));
    let settings = scc_context::ContextSettings {
        startup_tokens: config.context.startup_tokens,
        task_tokens: config.context.task_tokens,
        atlas_tokens: config.context.atlas_tokens,
        detail_tokens: config.context.detail_tokens,
        include_low_confidence_inference: config.context.include_low_confidence_inference,
        rank_salt: format!(
            "{}:{}:{}:{}",
            config.inference.enabled,
            config.inference.embedding_model,
            config.inference.rerank_model,
            plugin_salt,
        ),
        pack_allocator: scc_context::PackAllocator::AdaptivePriority,
    };
    Ok(Engine {
        store,
        graph,
        settings,
        stale,
    })
}

// trace:v1 id=impl.crates-scc-engine-src-workspace.engine work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
impl Engine<'_> {
    // trace:exempt reason=internal-detail
    pub fn ctx(&self) -> ContextCompiler<'_> {
        ContextCompiler::new(
            self.store,
            &self.graph,
            self.settings.clone(),
            self.stale.clone(),
        )
    }

    // trace:exempt reason=internal-detail
    pub fn context(&self) -> SccContext<'_> {
        SccContext { engine: self }
    }

    // trace:exempt reason=internal-detail
    pub fn ranking(&self) -> crate::ranking::Ranker<'_> {
        crate::ranking::Ranker::new(self)
    }

    // trace:exempt reason=internal-detail
    pub fn operations(&self) -> Operations<'_> {
        Operations { engine: self }
    }

    // trace:exempt reason=internal-detail
    pub fn invoke(&self, operation: &str, input: serde_json::Value) -> crate::Result<serde_json::Value> {
        crate::invoke::invoke(&self.store.root, operation, input)
    }
}

// trace:v1 id=impl.crates-scc-engine-src-workspace.stale-paths work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub fn stale_paths(store: &Store) -> crate::Result<Vec<String>> {
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
            Some(current) if *current == hash.as_str() => {}
            Some(_) => out.push(path),
            None => out.push(path),
        }
    }
    for f in scanned {
        if !indexed.contains(&f.path) {
            out.push(f.path);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[allow(dead_code)]
// trace:exempt reason=internal-detail
pub struct Operations<'a> {
    engine: &'a Engine<'a>,
}

// trace:v1 id=impl.crates-scc-engine-src-workspace.operations work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
impl Operations<'_> {
    // trace:exempt reason=internal-detail
    pub fn list(&self) -> &'static [crate::ops::OperationDescriptor] {
        crate::ops::OPERATIONS
    }

    // trace:exempt reason=internal-detail
    pub fn describe(&self, id: &str) -> Option<&'static crate::ops::OperationDescriptor> {
        crate::ops::describe(id)
    }
}

/// Pinned model session (§5): repository id, source revision, model epoch,
/// config hash, plugin lock, and rank salt captured at open.
///
/// Sessions let callers run atlas + ranking + structural against exactly
/// the same compiled model. `is_current()` re-checks the pin against live
/// state; a changed epoch/revision/config/plugin set reports stale instead
/// of silently answering from a moved model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.scc-engine-workspace.session work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct Session {
    pub repo_id: String,
    pub revision: String,
    pub epoch: String,
    pub config_hash: String,
    pub plugin_lock: Vec<serde_json::Value>,
    pub rank_salt: String,
}

// trace:exempt reason=internal-detail
pub fn open_session(store: &Store, config: &Config) -> crate::Result<Session> {
    let repo = store.repository();
    let head = store.revisions()?.into_iter().last().map(|r| r.rev).unwrap_or(0);
    let epoch = store.model_epoch()?.composite(&head.to_string());
    let revision = store
        .snapshot_status()?
        .map(|(s, _)| s.revision)
        .unwrap_or_else(|| "not-indexed".to_string());
    let engine = open_engine(store, config, stale_paths(store)?)?;
    Ok(Session {
        repo_id: repo.id,
        revision,
        epoch,
        config_hash: scc_indexer::semantic_config_hash(config),
        plugin_lock: crate::plugins::lock_entries(&crate::plugins::active(&store.root, config)),
        rank_salt: engine.settings.rank_salt,
    })
}

// trace:exempt reason=internal-detail
pub fn session_is_current(store: &Store, config: &Config, session: &Session) -> crate::Result<bool> {
    let live = open_session(store, config)?;
    Ok(live == *session)
}


