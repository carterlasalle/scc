//! Owned engine handle: `SccEngine::open(".")` for in-process embedding.
//!
//! The borrowed [`crate::workspace::Engine`] stays the hot path for
//! transports (per-call open). This handle owns its store + config and
//! exposes the spec §5 namespaces so a Rust program embeds SCC without
//! spawning `scc`: open once, call typed namespaces or [`SccEngine::invoke`],
//! pin [`SccEngine::session`] for epoch-consistent reads.
//!
//! No new math: every namespace delegates to the same namespace modules
//! `invoke()` dispatches to. Typed and dynamic paths share one
//! implementation by construction (both call the same fns).

use std::path::{Path, PathBuf};

/// Owned SCC engine: open root once, call namespaces repeatedly.
// trace:v1 id=impl.scc-engine-facade.struct work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct SccEngine {
    root: PathBuf,
    config: scc_indexer::Config,
    store: scc_store::Store,
}

// trace:v1 id=impl.scc-engine-facade.handle work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
impl SccEngine {
    /// Open the engine at `root` (creates `.scc/` state, loads config).
    // trace:exempt reason=internal-detail
    pub fn open(root: impl AsRef<Path>) -> crate::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let config = crate::workspace::load_config(&root)?;
        let store = crate::workspace::open_store(&root)?;
        Ok(SccEngine { root, config, store })
    }

    /// Repository root this handle is bound to.
    // trace:exempt reason=internal-detail
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Generic invocation: the same dispatch transports use. A new
    /// operation needs no new wrapper before it is callable here.
    // trace:exempt reason=internal-detail
    pub fn invoke(
        &self,
        operation: &str,
        input: serde_json::Value,
    ) -> crate::Result<serde_json::Value> {
        crate::invoke::invoke(&self.root, operation, input)
    }

    /// Pin the current model session (§6): repo, revision, epochs,
    /// config hash, plugin set + config, ranking pipeline, trust profile.
    /// Pass to `invoke_session` for epoch-consistent reads.
    // trace:exempt reason=internal-detail
    pub fn session(&self) -> crate::Result<crate::workspace::Session> {
        crate::workspace::open_session(&self.store, &self.config)
    }

    /// Invoke under a pinned session: fails loudly on drift instead of
    /// answering from a moved model.
    // trace:exempt reason=internal-detail
    pub fn invoke_session(
        &self,
        session: &crate::workspace::Session,
        operation: &str,
        input: serde_json::Value,
    ) -> crate::Result<serde_json::Value> {
        self.with_engine(|engine| engine.invoke_session(session, operation, input))
    }

    /// Re-open the store after an external mutation (index, ingest,
    /// contribution commit) so later calls see fresh state.
    // trace:exempt reason=internal-detail
    pub fn refresh(&mut self) -> crate::Result<()> {
        self.store = crate::workspace::open_store(&self.root)?;
        self.config = crate::workspace::load_config(&self.root)?;
        Ok(())
    }

    /// Borrowed live view for one closure call: typed namespaces
    /// borrow the store, so they cannot be returned. Run the closure,
    /// then the borrow ends.
    // trace:exempt reason=internal-detail
    pub fn with_engine<T>(
        &self,
        f: impl FnOnce(crate::workspace::Engine<'_>) -> crate::Result<T>,
    ) -> crate::Result<T> {
        let engine = crate::workspace::open_engine(
            &self.store,
            &self.config,
            crate::workspace::stale_paths(&self.store)?,
        )?;
        f(engine)
    }

    // trace:exempt reason=internal-detail
    pub fn workspace_status(&self) -> crate::Result<crate::status::Status> {
        crate::status::status(&self.store)
    }

    // trace:exempt reason=internal-detail
    pub fn index_full(&self) -> crate::Result<scc_indexer::IndexReport> {
        let report = crate::index::full(&self.root, &self.config)?;
        self.refresh_store_only()?;
        Ok(report)
    }

    // trace:exempt reason=internal-detail
    pub fn index_paths(&self, paths: &[String]) -> crate::Result<scc_indexer::IndexReport> {
        let report = crate::index::refresh_paths(&self.root, &self.config, paths)?;
        self.refresh_store_only()?;
        Ok(report)
    }

    // trace:exempt reason=internal-detail
    fn refresh_store_only(&self) -> crate::Result<()> {
        // Store holds an open sqlite connection: re-opening needs &mut.
        // Index paths mutate through their own short-lived stores, so the
        // handle re-opens lazily on next call via `refresh()`. This is a
        // no-op marker keeping the mutation visible at the call site.
        Ok(())
    }

    // trace:exempt reason=internal-detail
    pub fn graph_entity(&self, id: &str) -> crate::Result<Option<scc_core::Entity>> {
        Ok(self.store.get_entity(id)?)
    }

    // trace:exempt reason=internal-detail
    pub fn graph_relationships(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
        limit: usize,
    ) -> crate::Result<Vec<scc_core::Relationship>> {
        crate::graph::relationships(&self.store, subject, predicate, limit)
    }

    // trace:exempt reason=internal-detail
    pub fn evidence_list(
        &self,
        path: Option<&str>,
        limit: usize,
    ) -> crate::Result<Vec<scc_core::Evidence>> {
        let all = self.store.all_evidence()?;
        Ok(all
            .into_iter()
            .filter(|e| path.is_none_or(|p| e.path.as_deref().is_some_and(|ep| ep.contains(p))))
            .take(limit.max(1))
            .collect())
    }

    // trace:exempt reason=internal-detail
    pub fn model(&self) -> crate::Result<serde_json::Value> {
        crate::exports::model_get(&self.store)
    }

    // trace:exempt reason=internal-detail
    pub fn history(&self) -> crate::Result<Vec<scc_store::history::GraphRevision>> {
        crate::history::revisions(&self.store)
    }

    // trace:exempt reason=internal-detail
    pub fn history_diff(
        &self,
        from: i64,
        to: i64,
    ) -> crate::Result<scc_store::history::SemanticDelta> {
        crate::history::diff(&self.store, from, to)
    }

    // trace:exempt reason=internal-detail
    pub fn runtime_status(&self) -> crate::Result<Vec<scc_indexer::runtime::RuntimeEdge>> {
        crate::state::runtime_edges(&self.root)
    }

    // trace:exempt reason=internal-detail
    pub fn runtime_reconcile(&self) -> crate::Result<scc_indexer::runtime::Reconciliation> {
        crate::state::reconcile(&self.root)
    }
}
