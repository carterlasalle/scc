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

    /// Surface build for a task goal + budget (spec §5 usage):
    /// structured result plus rendered text. Same derivation as the
    /// `surface.build` operation (one builder, two callers).
    // trace:exempt reason=internal-detail
    pub fn surface_build(
        &self,
        task: &str,
        budget: usize,
        explain: bool,
    ) -> crate::Result<(scc_core::SurfaceRenderResult, String)> {
        self.with_engine(|engine| {
            let req = scc_api::SurfaceRequest {
                task: Some(task.to_string()),
                budget: Some(budget),
                explain,
                stages: None,
            };
            let (scorer, _) = crate::inference::rankers(engine.store, &self.config, task);
            let semantic: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            engine.context().surface(&req, semantic)
        })
    }

    /// Full per-symbol blend with feature decomposition (spec §5 usage):
    /// same math as `surface_build` minus MMR/quotas/budget. Goal +
    /// limit + explain map onto `RankRequest` exactly as the
    /// `ranking.symbols` operation builds it.
    // trace:exempt reason=internal-detail
    pub fn ranking_symbols(
        &self,
        goal: &str,
        limit: usize,
        explain: bool,
    ) -> crate::Result<scc_api::RankResult> {
        self.with_engine(|engine| {
            engine.ranking().symbols(&scc_api::RankRequest {
                goal: Some(goal.to_string()),
                profile: None,
                limit,
                explain,
                include_features: false,
                include_intermediate: false,
            })
        })
    }

    /// Task context artifact: pack + delta + ids + token count (spec §5
    /// usage). Same builder the `context.task` operation calls.
    // trace:exempt reason=internal-detail
    pub fn context_task(
        &self,
        goal: &str,
        budget: usize,
    ) -> crate::Result<crate::task::TaskContextArtifact> {
        self.with_engine(|engine| {
            let req = scc_api::TaskContextRequest {
                goal: goal.to_string(),
                files: vec![],
                symbols: vec![],
                budget: Some(budget),
                hook: false,
                record_visibility: true,
            };
            let (scorer, reranker) = crate::inference::rankers(engine.store, &self.config, goal);
            let scorer_trait: Option<&dyn scc_context::rank::SemanticScorer> =
                scorer.as_ref().map(|s| s as &dyn scc_context::rank::SemanticScorer);
            let reranker_trait: Option<&dyn scc_context::rank::Reranker> =
                reranker.as_ref().map(|r| r as &dyn scc_context::rank::Reranker);
            crate::task::build_task_context(&engine, &self.config, &self.root, &req, scorer_trait, reranker_trait)
        })
    }

    /// System atlas pack (spec §5 usage): same builder the
    /// `context.atlas` operation calls.
    // trace:exempt reason=internal-detail
    pub fn context_atlas(&self, budget: usize) -> crate::Result<scc_context::ContextPack> {
        self.with_engine(|engine| engine.context().atlas(Some(budget), false, false))
    }

    /// Spec §5 namespace chain: `scc.context()` — context packs behind a
    /// namespace view instead of one flat method list.
    // trace:exempt reason=internal-detail
    pub fn context_ns(&self) -> ContextNs<'_> {
        ContextNs { engine: self }
    }

    /// Spec §5 namespace chain: `scc.surface()`.
    // trace:exempt reason=internal-detail
    pub fn surface_ns(&self) -> SurfaceNs<'_> {
        SurfaceNs { engine: self }
    }

    /// Spec §5 namespace chain: `scc.ranking()`.
    // trace:exempt reason=internal-detail
    pub fn ranking_ns(&self) -> RankingNs<'_> {
        RankingNs { engine: self }
    }

    /// Spec §5 namespace chain: `scc.graph()`.
    // trace:exempt reason=internal-detail
    pub fn graph_ns(&self) -> GraphNs<'_> {
        GraphNs { engine: self }
    }
}

/// Namespace view: `scc.context_ns().atlas(..)` (spec §5 chain).
// trace:v1 id=impl.scc-engine-facade.context-ns work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct ContextNs<'a> {
    engine: &'a SccEngine,
}

// trace:exempt reason=internal-detail
impl ContextNs<'_> {
    // trace:exempt reason=internal-detail
    pub fn overview(&self) -> crate::Result<scc_context::ContextPack> {
        self.engine.with_engine(|e| e.context().overview())
    }
    // trace:exempt reason=internal-detail
    pub fn atlas(&self, budget: usize) -> crate::Result<scc_context::ContextPack> {
        self.engine.context_atlas(budget)
    }
    // trace:exempt reason=internal-detail
    pub fn atlas_model(
        &self,
        scope: scc_context::atlas::AtlasScope,
    ) -> crate::Result<scc_core::SystemAtlas> {
        self.engine.with_engine(|e| e.context().atlas_model(scope))
    }
    // trace:exempt reason=internal-detail
    pub fn task(
        &self,
        goal: &str,
        budget: usize,
    ) -> crate::Result<crate::task::TaskContextArtifact> {
        self.engine.context_task(goal, budget)
    }
}

/// Namespace view: `scc.surface_ns().build(..)` (spec §5 chain).
// trace:v1 id=impl.scc-engine-facade.surface-ns work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct SurfaceNs<'a> {
    engine: &'a SccEngine,
}

// trace:exempt reason=internal-detail
impl SurfaceNs<'_> {
    // trace:exempt reason=internal-detail
    pub fn build(
        &self,
        task: &str,
        budget: usize,
        explain: bool,
    ) -> crate::Result<(scc_core::SurfaceRenderResult, String)> {
        self.engine.surface_build(task, budget, explain)
    }
}

/// Namespace view: `scc.ranking_ns().symbols(..)` (spec §5 chain).
// trace:v1 id=impl.scc-engine-facade.ranking-ns work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct RankingNs<'a> {
    engine: &'a SccEngine,
}

// trace:exempt reason=internal-detail
impl RankingNs<'_> {
    // trace:exempt reason=internal-detail
    pub fn symbols(
        &self,
        goal: &str,
        limit: usize,
        explain: bool,
    ) -> crate::Result<scc_api::RankResult> {
        self.engine.ranking_symbols(goal, limit, explain)
    }
}

/// Namespace view: `scc.graph_ns().query(..)` (spec §5 chain).
// trace:v1 id=impl.scc-engine-facade.graph-ns work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct GraphNs<'a> {
    engine: &'a SccEngine,
}

// trace:exempt reason=internal-detail
impl GraphNs<'_> {
    // trace:exempt reason=internal-detail
    pub fn entity(&self, id: &str) -> crate::Result<Option<scc_core::Entity>> {
        self.engine.graph_entity(id)
    }
    // trace:exempt reason=internal-detail
    pub fn relationships(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
        limit: usize,
    ) -> crate::Result<Vec<scc_core::Relationship>> {
        self.engine.graph_relationships(subject, predicate, limit)
    }
    // trace:exempt reason=internal-detail
    pub fn query(
        &self,
        query: &str,
        limit: usize,
    ) -> crate::Result<crate::graph::QueryHit> {
        self.engine.with_engine(|_| {
            crate::graph::query(
                &self.engine.store,
                &scc_api::QueryRequest { query: query.to_string(), limit },
            )
        })
    }
}
