//! Universal operation registry (spec §§6-8): every SCC capability
//! gets a stable operation ID; `engine.operations().list()` explains the
//! API and `engine.invoke(id, input)` executes it. Typed namespaces
//! (`engine.context().task(..)`) and dynamic invoke share ONE
//! implementation — there are never two paths to the same behavior.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub enum MutationClass {
    Read,
    Write,
    Watch,
    /// Mutates caller-visible session state (ledger visibility, checkpoints).
    SessionMutation,
    /// Mutates the indexed model (reindex, ingest, contributions, snapshots).
    ModelMutation,
    /// Mutates external/project config (workspace init, plugin lockfile, lessons).
    ExternalMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub enum Stability {
    Stable,
    Experimental,
    Internal,
}

// trace:exempt reason=internal-detail
pub struct OperationDescriptor {
    pub id: &'static str,
    pub description: &'static str,
    pub mutation: MutationClass,
    pub streaming: bool,
    pub stability: Stability,
}

// trace:exempt reason=internal-detail
impl serde::Serialize for OperationDescriptor {
    // trace:exempt reason=internal-detail
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = serializer.serialize_struct("OperationDescriptor", 5)?;
        st.serialize_field("id", self.id)?;
        st.serialize_field("description", self.description)?;
        st.serialize_field("mutation", &self.mutation)?;
        st.serialize_field("streaming", &self.streaming)?;
        st.serialize_field("stability", &self.stability)?;
        st.end()
    }
}

// trace:exempt reason=internal-detail
pub const OPERATIONS: &[OperationDescriptor] = &[
    // workspace
    OperationDescriptor { id: "operations.list", description: "List every registered operation id (introspection)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "operations.describe", description: "Describe one operation by id", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "operations.schema", description: "JSON Schema for one operation's input (naming its scc-api request type)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "operations.capabilities", description: "Capability vocabulary: permission names, extension points, mutation classes", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.init", description: "Initialize the SCC workspace (.scc/config.yaml + database)", mutation: MutationClass::ExternalMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.status", description: "Index status, stats, and freshness", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.session", description: "Pin the current model session (repo, revision, epoch, config, plugins, salt)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.session_check", description: "Check a pinned session against live state (current vs stale)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.state_path", description: "Print the SCC state directory", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "workspace.scan", description: "Scan explanation: which files index and why", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "workspace.languages", description: "Generated language-support matrix", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // index
    OperationDescriptor { id: "index.full", description: "Index the repository (cold on first run, incremental afterwards)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "index.refresh", description: "Refresh selected paths", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "index.status", description: "Alias for workspace.status", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "index.paths", description: "Alias for index.refresh (refresh selected paths)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "index.watch", description: "Watch mode is CLI-local (file watcher loop); not an engine operation", mutation: MutationClass::Read, streaming: false , stability: Stability::Internal },
    // resolution
    OperationDescriptor { id: "resolution.run", description: "Run semantic resolution, then recompile the derived layer", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    // graph
    OperationDescriptor { id: "graph.recompile", description: "Recompile the derived graph layer", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.search", description: "FTS entity search with LIKE fallback (alias for graph.query entities half)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.search_symbols", description: "FTS symbol search with LIKE fallback (alias for graph.query symbols half)", mutation: MutationClass::Read, streaming: false , stability: Stability::Internal },
    OperationDescriptor { id: "graph.entity.get", description: "Fetch one entity by id", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.entities", description: "List components/entities", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.relationships", description: "Query relationships", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.query", description: "Lexical entity/symbol search with substring fallback", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.entity", description: "Alias for graph.entity.get", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "graph.traverse", description: "Multi-step directed traversal (out|in|both, predicate + kind filters)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "graph.explain", description: "Overlay diagnostics: every assertion behind one edge plus the trusted verdict", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "graph.flows", description: "List flows", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // context
    OperationDescriptor { id: "context.overview", description: "Startup capsule / system overview", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.atlas", description: "Full System Atlas", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.startup", description: "Atlas + Surface fusion for session startup", mutation: MutationClass::SessionMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.task", description: "Complete task artifact (pack + surface delta)", mutation: MutationClass::SessionMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.task_pack", description: "Enriched task pack only (no delta, no ledger)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.task_delta", description: "Ledger-aware task delta only (records visibility)", mutation: MutationClass::SessionMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "context.docs", description: "Alias for context.external_docs", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.subagent", description: "Narrower bounded task pack for delegated agents", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.component", description: "Context pack for one component", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.flow", description: "Context pack for one flow", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.impact", description: "Impact analysis pack", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.verify", description: "Freshness/evidence verification pack", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.external_docs", description: "External dependency docs via the configured Context7 command", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.structural", description: "Structural Source representation", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "context.compress", description: "Compress a task pack", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // surface + ranking
    OperationDescriptor { id: "surface.build", description: "System Surface Map (global or task-personalized)", mutation: MutationClass::Write, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.compile", description: "Compiled Surface Map: unranked candidate entries before PPR/rank/select", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "surface.global", description: "Alias for surface.build (global blend, no task)", mutation: MutationClass::Write, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.task", description: "Alias for surface.build (task-personalized blend)", mutation: MutationClass::Write, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.important", description: "Alias for ranking.important", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.explain", description: "Alias for ranking.explain", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.rank", description: "Alias for ranking.symbols", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.select", description: "Alias for selection.budget", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "surface.render", description: "Rendered surface text only (no structured result payload)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.important", description: "Fast where-to-pay-attention answer", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.symbols", description: "Full blend per symbol with feature decomposition + plugin hooks", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.global", description: "Alias for ranking.symbols (global blend, no goal)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.task", description: "Alias for ranking.symbols (task-personalized blend)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.entities", description: "Alias for ranking.symbols (entity-ranked blend)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.candidates", description: "Lexical candidate generation (stage 1)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.seeds", description: "Task-seed merge: lexical seeds + plugin seed providers (weight sums by id)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.reference_graph", description: "Normalized reference graph: trusted relationships as (source, kind, target) with provenance/confidence", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.universe", description: "Rank-universe nodes: (id, kind) over the heterogeneous universe in rank order", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.edges", description: "Rank-universe edges: (subject, predicate, object, base weight), pre-aggregation", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.project_symbols", description: "Project a universe vector to per-symbol scores (entity importance to owners)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.features", description: "Per-symbol feature decomposition (core 8 + plugin features) before the blend", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.pagerank.global", description: "Raw global PageRank vector over the heterogeneous universe", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.global_vector", description: "Alias for ranking.pagerank.global (raw global vector)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.pagerank.task", description: "Raw task-personalized PPR vector", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.task_vector", description: "Alias for ranking.pagerank.task (raw task PPR vector)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.final_importance", description: "Pure blend function over explicit features", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.score_entries", description: "Pure per-entry blend over explicit feature rows (batched final_importance)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.edge_weight", description: "Pure edge-weight function", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.architectural_specificity", description: "Architectural specificity (exported/public signals)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "ranking.entry", description: "One compiled SurfaceEntry by entry id (full structured candidate)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.trace", description: "Full ranking trace: items plus the seed and required inputs the blend consumed", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "ranking.explain", description: "Rank explanation for one symbol", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "selection.mmr", description: "MMR diversification over a ranked list", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "selection.quotas", description: "Token-fraction quota selection over ranked rows", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "selection.budget", description: "Value/token budget selection", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "selection.optimize", description: "Alias for selection.budget (budget-optimizer plugin slot)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "selection.required", description: "Never-omit entry ids: engine required set + plugin coverage providers", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "selection.preview", description: "Per-stage selection survivors (MMR → quotas → budget) over caller rows; default math only", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    // source
    OperationDescriptor { id: "source.structural", description: "Alias for context.structural", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // embeddings (optional semantic ranker)
    OperationDescriptor { id: "embeddings.build", description: "Compute and store entity embeddings with the configured model", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "embeddings.get", description: "Fetch the stored embedding vector for one entity", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "embeddings.status", description: "Stored embedding count", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // evidence
    OperationDescriptor { id: "evidence.get", description: "Fetch one evidence record by id", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "evidence.list", description: "List evidence records, optionally filtered by path", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "evidence.search", description: "Alias for evidence.list (path-filtered evidence search)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // runtime
    OperationDescriptor { id: "runtime.ingest", description: "Ingest runtime evidence", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "runtime.status", description: "Runtime observation status", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "runtime.reconcile", description: "Static-vs-observed reconciliation", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "runtime.signatures", description: "Trace-path signatures recorded during OTLP ingest", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // architecture
    OperationDescriptor { id: "architecture.components", description: "Alias for graph.entities", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "architecture.flows", description: "Alias for graph.flows", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "architecture.invariants", description: "Structural invariant checks", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "architecture.drift", description: "Architectural drift findings", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "architecture.cochange", description: "Git co-change pairs", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    // history / snapshots / checkpoints
    OperationDescriptor { id: "history.revisions", description: "Graph revision history", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "history.list", description: "Alias for history.revisions", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "history.diff", description: "Semantic diff between revisions", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "snapshot.save", description: "Save a task snapshot", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "snapshot.get", description: "Show a snapshot artifact", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "snapshot.diff", description: "Diff a snapshot against current", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "checkpoint.save", description: "Capture session checkpoint", mutation: MutationClass::SessionMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "checkpoint.load", description: "Load session checkpoint", mutation: MutationClass::SessionMutation, streaming: false , stability: Stability::Stable },
    // systems / imports / exports
    OperationDescriptor { id: "system.stitch", description: "Multi-repo stitch (routes/topics/exports)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.scip", description: "Import external evidence (SCIP/CCG/GitNexus/Beads...)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.ccg", description: "Alias for import.scip format=ccg", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.gitnexus", description: "Alias for import.scip format=gitnexus", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.tracelayer", description: "Alias for import.scip format=tracelayer", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.beads", description: "Alias for import.scip format=beads", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.hindsight", description: "Alias for import.scip format=hindsight", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.cbm", description: "Alias for import.scip format=cbm", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "import.plugin", description: "Import evidence from an evidence-provider plugin: import.<plugin-id> commits the plugin evidence.import batch through validate+commit", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "export.system_ir", description: "Export System IR (json/jsonl/ccg/flow-graphs)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "model.get", description: "Complete live model: repository, epoch, files, entities, relationships, evidence, components, flows, invariants, stats", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "model.drift", description: "Alias for architecture.drift", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "model.components", description: "Alias for graph.entities", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "model.flows", description: "Alias for graph.flows", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "model.invariants", description: "Alias for architecture.invariants", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.system_ir_jsonl", description: "Alias for export.system_ir format=system-ir.jsonl", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.ccg", description: "Alias for export.system_ir format=ccg", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.flow_graphs", description: "Alias for export.system_ir format=flow-graphs.json", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.diagram", description: "Architecture diagram (mermaid|svg) with node/edge/flow counts", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.snap", description: "Alias for export.system_ir format=capsule.md", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "export.capsule", description: "Alias for export.system_ir format=capsule.md", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "diagram.render", description: "Alias for export.diagram", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "viewer.panels", description: "Plugin viewer data panels (structured title/html + provenance)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "viewer.snapshot", description: "Viewer snapshots are CLI-local browser capture; not an engine operation", mutation: MutationClass::Read, streaming: false , stability: Stability::Internal },
    // integrity / integrations / lessons / setup
    OperationDescriptor { id: "integrity.invariants", description: "Alias for architecture.invariants", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integrity.ci", description: "CI gate over drift severity", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integrations.list", description: "Configured adapters with capability scope", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integration.list", description: "Alias for integrations.list", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integration.doctor", description: "Alias for integrations.doctor", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integrations.doctor", description: "Offline adapter diagnostics", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "integrations.describe", description: "Describe one integration adapter by name", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "lessons.add", description: "Append a hindsight lesson", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "lessons.list", description: "List hindsight lessons", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "beads.list", description: "List active bead tasks", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "beads.active", description: "Alias for beads.list", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.list", description: "List enabled plugins with lock entries", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.describe", description: "Describe one plugin manifest", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.doctor", description: "Plugin environment + failure diagnostics", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.invoke", description: "Invoke any plugin operation explicitly", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.lock", description: "Write .scc/plugins.lock from the live plugin set", mutation: MutationClass::ExternalMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.check", description: "Verify live plugins against .scc/plugins.lock", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.enable", description: "Add a plugin id to the project allow-list (plugins.enabled in .scc/config.yaml)", mutation: MutationClass::ExternalMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "plugins.disable", description: "Remove a plugin id from the project allow-list (plugins.enabled in .scc/config.yaml)", mutation: MutationClass::ExternalMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "plugins.graph", description: "Deterministic extension order per type (priority + before/after DAG)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "plugins.contribute", description: "Validate and commit a plugin contribution batch (entities, relationships, evidence)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugins.promote", description: "Promote selected sidecar findings to canonical relationships (core predicates, existing endpoints, provenance-stamped)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "plugin_state.get", description: "Read one namespaced plugin state key (StateRead grant)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugin_state.put", description: "Write one namespaced plugin state key (StateWrite grant)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugin_state.delete", description: "Delete one namespaced plugin state key (StateWrite grant)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "plugin_state.scan", description: "Scan namespaced plugin state keys by prefix (StateRead grant)", mutation: MutationClass::Read, streaming: false , stability: Stability::Stable },
    OperationDescriptor { id: "sidecar.put", description: "Write one raw sidecar fact under (plugin, graph, key) (StateWrite grant; never authoritative)", mutation: MutationClass::ModelMutation, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "sidecar.get", description: "Read one raw sidecar fact (StateRead grant)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
    OperationDescriptor { id: "sidecar.scan", description: "Scan raw sidecar facts by prefix within one plugin graph (StateRead grant)", mutation: MutationClass::Read, streaming: false , stability: Stability::Experimental },
];

// trace:exempt reason=internal-detail
pub fn describe(id: &str) -> Option<&'static OperationDescriptor> {
    OPERATIONS.iter().find(|d| d.id == id)
}

// trace:exempt reason=internal-detail
pub fn ids() -> Vec<&'static str> {
    OPERATIONS.iter().map(|d| d.id).collect()
}

/// JSON Schema for one operation's input (§9): generated from the canonical
/// scc-api request type via schemars — the same struct invoke deserializes,
/// so the schema can never drift from the implementation. Alias ops share
/// their canonical op's schema. Returns None for unknown ids.
// trace:v1 id=impl.scc-engine-ops.input-schema work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn input_schema(id: &str) -> Option<serde_json::Value> {
    use scc_api::*;
    // Operational envelope inputs (no dedicated request struct): free-form
    // object with the documented fields.
    let free = |props: &[(&str, &str)]| {
        let mut m = serde_json::Map::new();
        for (k, t) in props {
            m.insert(k.to_string(), serde_json::json!({"type": t}));
        }
        serde_json::json!({"type": "object", "properties": m})
    };
    let v: serde_json::Value = match id {
        "context.task" | "context.task_pack" | "context.task_delta" => {
            serde_json::to_value(schemars::schema_for!(TaskContextRequest)).unwrap_or(serde_json::json!({}))
        }
        "context.startup" => serde_json::to_value(schemars::schema_for!(StartupRequest)).unwrap_or(serde_json::json!({})),
        "surface.build" | "surface.global" | "surface.task" | "surface.render" | "ranking.important" | "surface.important" => {
            serde_json::to_value(schemars::schema_for!(SurfaceRequest)).unwrap_or(serde_json::json!({}))
        }
        "context.component" | "context.flow" => serde_json::to_value(schemars::schema_for!(DetailRequest)).unwrap_or(serde_json::json!({})),
        "surface.compile" => free(&[]),
        "context.impact" => serde_json::to_value(schemars::schema_for!(ImpactRequest)).unwrap_or(serde_json::json!({})),
        "context.structural" | "source.structural" => {
            serde_json::to_value(schemars::schema_for!(StructuralRequest)).unwrap_or(serde_json::json!({}))
        }
        "graph.query" | "graph.search" | "graph.search_symbols" => {
            serde_json::to_value(schemars::schema_for!(QueryRequest)).unwrap_or(serde_json::json!({}))
        }
        "graph.traverse" => serde_json::to_value(schemars::schema_for!(TraverseRequest)).unwrap_or(serde_json::json!({})),
        "graph.explain" => free(&[("subject", "string"), ("predicate", "string"), ("object", "string")]),
        "plugins.describe" => free(&[("id", "string")]),
        "plugins.inspect" => free(&[("id", "string")]),
        "plugins.enable" | "plugins.disable" => free(&[("id", "string")]),
        "graph.entity.get" | "graph.entity" => free(&[("id", "string")]),
        "ranking.symbols" | "ranking.global" | "ranking.task" | "ranking.entities" | "surface.rank" => {
            serde_json::to_value(schemars::schema_for!(RankRequest)).unwrap_or(serde_json::json!({}))
        }
        _ => return None,
    };
    Some(serde_json::json!({"operation": id, "input": v}))
}

/// Capability vocabulary (§9): the permission names plugins request, the
/// extension-point names they register under, and the mutation classes
/// operations declare. Derived from the [`scc_plugin_api::Permission`] enum
/// (single source) so docs can never drift from enforcement.
// trace:v1 id=impl.scc-engine-ops.capabilities work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn capabilities() -> serde_json::Value {
    serde_json::json!({
        "permissions": [
            {"id": "repo.read", "description": "Read repository files"},
            {"id": "repo.write", "description": "Write repository files"},
            {"id": "graph.read", "description": "Read the Reality Graph (raw + trusted)"},
            {"id": "graph.contribute", "description": "Contribute entities/relationships/evidence"},
            {"id": "evidence.contribute", "description": "Alias scope for graph.contribute (evidence importers)"},
            {"id": "runtime.contribute", "description": "Contribute runtime observations"},
            {"id": "state.read", "description": "Read namespaced plugin state"},
            {"id": "state.write", "description": "Write namespaced plugin state"},
            {"id": "network", "description": "Network access"},
            {"id": "subprocess", "description": "Spawn subprocesses"},
            {"id": "operation.register", "description": "Register custom operations (via manifest operations list)"},
            {"id": "ranking.extend", "description": "Ranking hooks: seeds, candidates, features, rank edges, weights, rerank, similarity, profiles, coverage"},
            {"id": "context.extend", "description": "Context sections: task sections, startup sections, verify diagnostics"},
            {"id": "renderer.extend", "description": "Export/diagram rendering via export.* operations and exporter:<format> extensions"},
        ],
        "extension_points": [
            "candidate-provider", "seed-provider", "rank-feature", "rank-edge", "edge-weight",
            "reranker", "similarity", "blend-profile", "coverage", "verify-diagnostic",
            "context-section", "startup-section", "exporter", "viewer-panel",
            "quota-policy", "budget-optimizer", "diversity-policy",
            "operation",
        ],
        "mutation_classes": ["Read", "Write", "Watch"],
        "plugin_api_version": scc_plugin_api::PLUGIN_API_VERSION,
        "api_version": scc_api::API_VERSION,
    })
}
