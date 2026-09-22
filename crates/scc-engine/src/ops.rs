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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct OperationDescriptor {
    pub id: &'static str,
    pub description: &'static str,
    pub mutation: MutationClass,
    pub streaming: bool,
}

// trace:exempt reason=internal-detail
pub const OPERATIONS: &[OperationDescriptor] = &[
    // workspace
    OperationDescriptor { id: "workspace.init", description: "Initialize the SCC workspace (.scc/config.yaml + database)", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "workspace.status", description: "Index status, stats, and freshness", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "workspace.state_path", description: "Print the SCC state directory", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "workspace.languages", description: "Generated language-support matrix", mutation: MutationClass::Read, streaming: false },
    // index
    OperationDescriptor { id: "index.full", description: "Index the repository (cold on first run, incremental afterwards)", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "index.refresh", description: "Refresh selected paths", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "index.status", description: "Alias for workspace.status", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "index.watch", description: "Watch the filesystem and re-index changed files", mutation: MutationClass::Watch, streaming: true },
    // resolution
    OperationDescriptor { id: "resolution.run", description: "Run semantic resolution, then recompile the derived layer", mutation: MutationClass::Write, streaming: false },
    // graph
    OperationDescriptor { id: "graph.recompile", description: "Recompile the derived graph layer", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "graph.entity.get", description: "Fetch one entity by id", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "graph.entities", description: "List components/entities", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "graph.relationships", description: "Query relationships", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "graph.query", description: "Lexical entity/symbol search with substring fallback", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "graph.flows", description: "List flows", mutation: MutationClass::Read, streaming: false },
    // context
    OperationDescriptor { id: "context.overview", description: "Startup capsule / system overview", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.atlas", description: "Full System Atlas", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.startup", description: "Atlas + Surface fusion for session startup", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "context.task", description: "Complete task artifact (pack + surface delta)", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "context.task_pack", description: "Enriched task pack only (no delta, no ledger)", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.subagent", description: "Narrower bounded task pack for delegated agents", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.component", description: "Context pack for one component", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.flow", description: "Context pack for one flow", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.impact", description: "Impact analysis pack", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.verify", description: "Freshness/evidence verification pack", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.structural", description: "Structural Source representation", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "context.compress", description: "Compress a task pack", mutation: MutationClass::Read, streaming: false },
    // surface + ranking
    OperationDescriptor { id: "surface.build", description: "System Surface Map (global or task-personalized)", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "ranking.important", description: "Fast where-to-pay-attention answer", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.symbols", description: "Full blend per symbol with feature decomposition + plugin hooks", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.candidates", description: "Lexical candidate generation (stage 1)", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.pagerank.global", description: "Raw global PageRank vector over the heterogeneous universe", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.pagerank.task", description: "Raw task-personalized PPR vector", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.final_importance", description: "Pure blend function over explicit features", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.edge_weight", description: "Pure edge-weight function", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.architectural_specificity", description: "Architectural specificity (exported/public signals)", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "ranking.explain", description: "Rank explanation for one symbol", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "selection.mmr", description: "MMR diversification over a ranked list", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "selection.quotas", description: "Token-fraction quota selection over ranked rows", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "selection.budget", description: "Value/token budget selection", mutation: MutationClass::Read, streaming: false },
    // source
    OperationDescriptor { id: "source.structural", description: "Alias for context.structural", mutation: MutationClass::Read, streaming: false },
    // evidence / runtime
    OperationDescriptor { id: "runtime.ingest", description: "Ingest runtime evidence", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "runtime.status", description: "Runtime observation status", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "runtime.reconcile", description: "Static-vs-observed reconciliation", mutation: MutationClass::Read, streaming: false },
    // architecture
    OperationDescriptor { id: "architecture.components", description: "Alias for graph.entities", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "architecture.flows", description: "Alias for graph.flows", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "architecture.invariants", description: "Structural invariant checks", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "architecture.drift", description: "Architectural drift findings", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "architecture.cochange", description: "Git co-change pairs", mutation: MutationClass::Read, streaming: false },
    // history / snapshots / checkpoints
    OperationDescriptor { id: "history.revisions", description: "Graph revision history", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "history.diff", description: "Semantic diff between revisions", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "snapshot.save", description: "Save a task snapshot", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "snapshot.get", description: "Show a snapshot artifact", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "snapshot.diff", description: "Diff a snapshot against current", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "checkpoint.save", description: "Capture session checkpoint", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "checkpoint.load", description: "Load session checkpoint", mutation: MutationClass::Read, streaming: false },
    // systems / imports / exports
    OperationDescriptor { id: "system.stitch", description: "Multi-repo stitch (routes/topics/exports)", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "import.scip", description: "Import external evidence (SCIP/CCG/GitNexus/Beads...)", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "export.system_ir", description: "Export System IR (json/jsonl/ccg/flow-graphs)", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "export.diagram", description: "Render architecture diagram", mutation: MutationClass::Read, streaming: false },
    // integrity / integrations / lessons / setup
    OperationDescriptor { id: "integrity.invariants", description: "Alias for architecture.invariants", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "integrity.ci", description: "CI gate over drift severity", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "integrations.list", description: "Configured adapters with capability scope", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "integrations.doctor", description: "Offline adapter diagnostics", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "lessons.add", description: "Append a hindsight lesson", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "lessons.list", description: "List hindsight lessons", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "plugins.list", description: "List enabled plugins with lock entries", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "plugins.describe", description: "Describe one plugin manifest", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "plugins.doctor", description: "Plugin environment + failure diagnostics", mutation: MutationClass::Read, streaming: false },
    OperationDescriptor { id: "plugins.invoke", description: "Invoke any plugin operation explicitly", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "setup.claude", description: "Install Claude integration", mutation: MutationClass::Write, streaming: false },
    OperationDescriptor { id: "setup.detected", description: "Install all detected harness integrations", mutation: MutationClass::Write, streaming: false },
];

// trace:exempt reason=internal-detail
pub fn describe(id: &str) -> Option<&'static OperationDescriptor> {
    OPERATIONS.iter().find(|d| d.id == id)
}

// trace:exempt reason=internal-detail
pub fn ids() -> Vec<&'static str> {
    OPERATIONS.iter().map(|d| d.id).collect()
}

