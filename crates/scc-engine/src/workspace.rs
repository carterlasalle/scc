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
pub fn open_engine<'a>(
    store: &'a Store,
    config: &Config,
    stale: Vec<String>,
) -> crate::Result<Engine<'a>> {
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
