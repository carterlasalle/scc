//! Graph layer: Reality Graph loading plus the System IR compilers
//! (components, flows, invariants) and impact analysis.
//!
//! Docs mapping: scc-graph + scc-system-ir + scc-flow.

pub mod archetype;
pub mod boundaries;
pub mod clustering;
pub mod cochange;
pub mod components;
pub mod flowgraph;
pub mod flows;
pub mod impact;
pub mod invariants;
pub mod lifecycle;
pub mod state;
pub mod trust;
pub mod workflow;

pub use trust::{TrustedGraphView, TrustPolicy};

// trace:v1 id=impl.scc-graph.reality-graph work=WORK-SCC-004 satisfies=REQ-SCC-IR
impl RealityGraph {
    pub fn empty() -> RealityGraph {
        RealityGraph {
            repo_id: String::new(),
            entities: HashMap::new(),
            out: HashMap::new(),
            inn: HashMap::new(),
            components: Vec::new(),
            flows: Vec::new(),
            invariants: Vec::new(),
        }
    }
}

use scc_core::{Entity, Flow, Invariant, Relationship};
use scc_store::Store;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
// trace:exempt reason=internal-detail
pub enum GraphError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
    #[error("cochange: {0}")]
    Cochange(String),
    #[error("impact: {0}")]
    Impact(String),
}

pub type Result<T> = std::result::Result<T, GraphError>;

/// In-memory view of the reality graph.
// trace:v1 id=impl.scc-graph.reality-graph-struct work=WORK-SCC-004 satisfies=REQ-SCC-IR
pub struct RealityGraph {
    pub repo_id: String,
    pub entities: HashMap<String, Entity>,
    /// out edges by subject
    pub out: HashMap<String, Vec<Relationship>>,
    /// in edges by object
    pub inn: HashMap<String, Vec<Relationship>>,
    pub components: Vec<Entity>,
    pub flows: Vec<Flow>,
    pub invariants: Vec<Invariant>,
}

impl RealityGraph {
    pub fn load(store: &Store) -> Result<RealityGraph> {
        let mut entities = HashMap::new();
        for e in store.all_entities()? {
            entities.insert(e.id.clone(), e);
        }
        let mut out: HashMap<String, Vec<Relationship>> = HashMap::new();
        let mut inn: HashMap<String, Vec<Relationship>> = HashMap::new();
        for r in store.all_relationships()? {
            out.entry(r.subject.clone()).or_default().push(r.clone());
            inn.entry(r.object.clone()).or_default().push(r);
        }
        Ok(RealityGraph {
            repo_id: store.repo_id.clone(),
            entities,
            out,
            inn,
            components: store.components()?,
            flows: store.flows()?,
            invariants: store.invariants()?,
        })
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn out_edges(&self, id: &str) -> Vec<&Relationship> {
        self.out.get(id).map(|v| v.iter().collect()).unwrap_or_default()
    }

    pub fn in_edges(&self, id: &str) -> Vec<&Relationship> {
        self.inn.get(id).map(|v| v.iter().collect()).unwrap_or_default()
    }

    pub fn out_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.out
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn in_pred(&self, id: &str, predicate: &str) -> Vec<&Relationship> {
        self.inn
            .get(id)
            .map(|v| {
                v.iter()
                    .filter(|r| r.predicate == predicate)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Entities of a given kind (sorted by name for determinism).
    pub fn entities_of_kind(&self, kind: &str) -> Vec<&Entity> {
        let mut v: Vec<&Entity> = self
            .entities
            .values()
            .filter(|e| e.kind == kind)
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }
}

/// Map every symbol id to its component id via the component CONTAINS
/// edges (shared by the flow graph compiler and flow projections).
/// Affected closure for scoped recompilation (C1b): the set of
/// derived entities that may have changed given source-file mutations.
/// File -> owning component(s) via stored component CONTAINS file edges;
/// component -> flows via flow participant edges. Conservative: unknown
/// files map to the whole closure (None = full recompile required).
// trace:v1 id=impl.scc-graph-affected-closure work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct AffectedClosure {
    /// Component ids that may have changed (empty + `complete=true` means all).
    pub components: Vec<String>,
    /// Flow ids that may have changed.
    pub flows: Vec<String>,
    /// True when the closure cannot be bounded (unknown file, topology
    /// change): the caller must run the full pipeline.
    pub complete: bool,
}

// trace:v1 id=impl.scc-graph-affected-closure-fn work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn affected_closure_with_owners(
    graph: &RealityGraph,
    changed_files: &[String],
    known_owners: &[String],
) -> AffectedClosure {
    use std::collections::HashSet;
    // Pre-purge owners (captured by the indexer while previous-gen edges
    // existed) are authoritative: seed the component set directly, then
    // expand to flows below. Unknown files with no known owner stay
    // unbounded.
    let mut components: HashSet<String> = HashSet::new();
    for o in known_owners {
        components.insert(o.clone());
    }
    let inner = affected_closure(graph, changed_files);
    if inner.complete && !components.is_empty() {
        // Owners known despite unmappable files: bounded after all.
        let mut flows: HashSet<String> = HashSet::new();
        let sym_comp = symbol_component_map(graph);
        let comp_set: HashSet<&str> = components.iter().map(|c| c.as_str()).collect();
        for fl in &graph.flows {
            for r in graph.out_pred(&fl.id, scc_core::predicates::CONTAINS) {
                if comp_set.contains(r.object.as_str())
                    || sym_comp.get(&r.object).map(|c| comp_set.contains(c.as_str())).unwrap_or(false)
                {
                    flows.insert(fl.id.clone());
                    break;
                }
            }
        }
        let mut components: Vec<String> = components.into_iter().collect();
        components.sort();
        let mut flows: Vec<String> = flows.into_iter().collect();
        flows.sort();
        return AffectedClosure { components, flows, complete: false };
    }
    if !inner.complete {
        for c in &inner.components {
            components.insert(c.clone());
        }
    }
    if components.is_empty() {
        return inner;
    }
    // Recompute flows over the union.
    let sym_comp = symbol_component_map(graph);
    let comp_set: HashSet<&str> = components.iter().map(|c| c.as_str()).collect();
    let mut flows: HashSet<String> = HashSet::new();
    for fl in &graph.flows {
        for r in graph.out_pred(&fl.id, scc_core::predicates::CONTAINS) {
            if comp_set.contains(r.object.as_str())
                || sym_comp.get(&r.object).map(|c| comp_set.contains(c.as_str())).unwrap_or(false)
            {
                flows.insert(fl.id.clone());
                break;
            }
        }
    }
    let mut components: Vec<String> = components.into_iter().collect();
    components.sort();
    let mut flows: Vec<String> = flows.into_iter().collect();
    flows.sort();
    AffectedClosure { components, flows, complete: false }
}

// trace:v1 id=impl.scc-graph-affected-closure-base work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn affected_closure(graph: &RealityGraph, changed_files: &[String]) -> AffectedClosure {
    use std::collections::HashSet;
    // NOTE: the closure runs pre-recompile against the PREVIOUS generation's
    // component edges. A file the previous clustering never placed (new
    // files, unplaced files) cannot map to an owner — that is correct
    // invalidation semantics (placement is unknowable until clustering
    // runs), and correctly forces complete=true. Do not "fix" by falling
    // back to neighbor heuristics: an unbounded closure is the honest
    // answer when ownership is unknown.
    let mut components: HashSet<String> = HashSet::new();
    // file entity id prefix: files are entities `repo:{repo_id}/file/{path}`.
    // Symbol -> component owner via the same CONTAINS edges (file CONTAINS
    // symbol, component CONTAINS file): survives the refresh purge, which
    // deletes the changed file's direct edges (source_path) before the
    // closure runs. File ids end with `/{path}`.
    let mut sym_owner: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for c in &graph.components {
        for r in graph.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            for sr in graph.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                sym_owner.insert(sr.object.clone(), c.id.clone());
            }
        }
    }
    for f in changed_files {
        let mut found = false;
        for c in &graph.components {
            for r in graph.out_pred(&c.id, scc_core::predicates::CONTAINS) {
                // Component CONTAINS file-entity ids; file ids end with
                // the path (`repo:{id}/file/{path}`) or equal it.
                if r.object == *f || r.object.ends_with(&format!("/{f}")) {
                    components.insert(c.id.clone());
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            // Purge-safe fallback: any symbol still owned whose id embeds
            // the file path (`.../symbol/{path}/{name}`) proves the file's
            // previous owner. New files with no symbols stay unbounded.
            for (sym, owner) in &sym_owner {
                if sym.contains(&format!("/symbol/{f}/")) || sym.ends_with(&format!("/symbol/{f}")) {
                    components.insert(owner.clone());
                    found = true;
                    break;
                }
            }
        }
        if !found {
            // Unknown file (new path, unmapped): cannot bound the closure.
            return AffectedClosure { components: vec![], flows: vec![], complete: true };
        }
    }
    // Component -> flows: flows whose participant set intersects the
    // affected components (via flow CONTAINS component or symbol edges
    // resolving through symbol_component_map).
    let sym_comp = symbol_component_map(graph);
    let comp_set: HashSet<&str> = components.iter().map(|c| c.as_str()).collect();
    let mut flows: HashSet<String> = HashSet::new();
    for fl in &graph.flows {
        let mut hit = false;
        for r in graph.out_pred(&fl.id, scc_core::predicates::CONTAINS) {
            if comp_set.contains(r.object.as_str()) {
                hit = true;
                break;
            }
            if let Some(c) = sym_comp.get(&r.object) {
                if comp_set.contains(c.as_str()) {
                    hit = true;
                    break;
                }
            }
        }
        if hit {
            flows.insert(fl.id.clone());
        }
    }
    let mut components: Vec<String> = components.into_iter().collect();
    components.sort();
    let mut flows: Vec<String> = flows.into_iter().collect();
    flows.sort();
    AffectedClosure { components, flows, complete: false }
}

pub fn symbol_component_map(graph: &RealityGraph) -> HashMap<String, String> {
    let mut symbol_comp: HashMap<String, String> = HashMap::new();
    for c in &graph.components {
        for r in graph.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            for sr in graph.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                symbol_comp.insert(sr.object.clone(), c.id.clone());
            }
        }
    }
    symbol_comp
}

/// Staged derived compilation (P0, docs/SYSTEM_DESIGN.md §7): every stage
/// writes its output, reloads the reality graph, and only then compiles the
/// next stage, so drift and later stages can never be computed against a
/// graph that predates freshly written facts. The derived model epoch is
/// bumped *before* the first write so cached context packs are invalidated
/// even if a stage fails mid-pipeline (fail closed — no stale trusted pack
/// survives a partial recompile).
// trace:v1 id=impl.scc-graph.compilation-pipeline work=WORK-SCC-004 satisfies=REQ-SCC-IR
pub struct CompilationPipeline<'a> {
    store: &'a Store,
    component_signals: Vec<components::ComponentSignal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationStage {
    Components,
    Flows,
    Behavior,
    Invariants,
    Drift,
    Boundaries,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageCounts {
    pub components: usize,
    pub flows: usize,
    pub invariants: usize,
    pub drift: usize,
    pub boundaries: usize,
}

// trace:exempt reason=internal-detail
impl<'a> CompilationPipeline<'a> {
    // trace:exempt reason=internal-detail
    pub fn new(store: &'a Store) -> CompilationPipeline<'a> {
        CompilationPipeline { store, component_signals: Vec::new() }
    }

    /// Plugin component signals (§31 ComponentSignalProvider): merged by
    /// name into the builtin candidate set before clustering. Same-name
    /// signal dirs append (never rename or re-rank); new names enter at
    /// plugin rank; failures upstream degrade to empty (never fail here).
    // trace:exempt reason=internal-detail
    pub fn component_signals(mut self, signals: Vec<components::ComponentSignal>) -> Self {
        self.component_signals = signals;
        self
    }

    // trace:exempt reason=internal-detail
    pub fn run(self) -> Result<RecompileReport> {
        // invalidate epoch-keyed context caches before any derived write
        self.store
            .bump_epoch(scc_store::ModelEpochKind::Derived)?;

        // STAGE 0/1: load the base reality graph, compile components.
        // Co-change pairs are computed first (Wave 5): they feed the
        // clustering score during compilation and enrich the freshly
        // written components right after, before any later stage reloads
        // the graph. Not a git repo yields empty pairs — no signal, no
        // error.
        let graph = RealityGraph::load(self.store)?;
        let intent = self.store.intent_claims()?;
        // HEAD-keyed cache in store meta: a cache hit skips the git pass
        // entirely (second `scc index` at the same HEAD is near-instant);
        // a miss computes with per-commit caps and persists. Any failure
        // is non-fatal — empty pairs, same as a non-git repo — so the
        // atlas/index never waits on git history.
        let pairs = cochange::cached_cochange_pairs(self.store).unwrap_or_default();
        let comps = components::compile_components_with_signals(&graph, self.store, &intent, &pairs, &self.component_signals)?;
        self.store.replace_components(&comps)?;
        cochange::enrich_components(self.store, &pairs).map_err(GraphError::Cochange)?;

        // STAGE 2: reload (components now visible), compile flows.
        let graph = RealityGraph::load(self.store)?;
        let (seq_flows, data_flows, arch_flow) =
            flows::compile_flows(&graph, self.store, &intent)?;
        let mut all = seq_flows;
        all.extend(data_flows);
        if let Some(a) = arch_flow {
            all.push(a);
        }
        self.store.replace_flows(&all)?;

        // STAGE 3: canonical causal flow graphs (Wave 3) — the behavioral
        // truth from which projections derive; then the behavioral views
        // (lifecycle state machines + operational workflows) which read the
        // stored sequence flows (reload).
        let graph = RealityGraph::load(self.store)?;
        let symbol_comp = symbol_component_map(&graph);
        let graphs = flowgraph::compile_flow_graphs(&graph, self.store, &intent, &symbol_comp)?;
        self.store.replace_flow_graphs(&graphs)?;
        let graph = RealityGraph::load(self.store)?;
        let mut lifecycles = lifecycle::compile_lifecycles(&graph, self.store)?;
        let mut workflows = workflow::compile_workflows(&graph, self.store)?;
        all.append(&mut lifecycles);
        all.append(&mut workflows);
        self.store.replace_flows(&all)?;

        // STAGE 4: invariants against the fully compiled model.
        let graph = RealityGraph::load(self.store)?;
        let invs = invariants::compile_invariants(&graph, &intent)?;
        self.store.replace_invariants(&invs)?;

        // STAGE 5: drift against the *stored* components (reloaded), never
        // the pre-reload in-memory list.
        let graph = RealityGraph::load(self.store)?;
        let stored_comps = self.store.components()?;
        let findings = invariants::drift_findings(&graph, self.store, &intent, &stored_comps)?;
        self.store.clear_drift_findings()?;
        for (kind, severity, message) in &findings {
            self.store
                .add_drift_finding(kind, severity, message)?;
        }

        // STAGE 6: trust-boundary crossings (derived facts).
        let graph = RealityGraph::load(self.store)?;
        let crossings = boundaries::compile_boundaries(&graph, self.store)?;
        for (rel, src) in crossings {
            self.store.insert_relationship(&rel, &src)?;
        }

        // garbage-collect evidence that lost its last reference during the
        // rebuild (docs/DATA_STRATEGY.md §6)
        self.store.sweep_orphan_evidence()?;
        // dangling-edge sweep: an edge whose endpoint has no entity (e.g. an
        // import resolved to a path whose own write was skipped) would fail
        // `scc check-invariants` forever. Same predicate as the checker, in
        // the same pass that rebuilds the derived layer.
        let _ = self.store.sweep_dangling_edges()?;

        Ok(RecompileReport {
            components: comps.len(),
            flows: all.len(),
            invariants: invs.len(),
            drift: findings.len(),
            boundaries: stored_comps.len(),
        })
    }
}

/// Scoped recompile entry (C1b): derive the affected closure for the
/// changed files and recompile. When the closure is bounded, only the
/// affected components/flows are recorded in the report; the pipeline
/// itself still runs whole-repo until per-stage merge writes land — the
/// closure is the contract that merge work will consume. `complete=true`
/// (unbounded) always runs the full pipeline.
/// Returns the closure alongside the report so callers can observe how
/// tight the bound was.
// trace:v1 id=impl.scc-graph-recompile-scoped work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn recompile_scoped(
    store: &Store,
    changed_files: &[String],
    component_signals: Vec<components::ComponentSignal>,
) -> Result<(RecompileReport, AffectedClosure)> {
    recompile_scoped_with_owners(store, changed_files, &[], component_signals)
}

// trace:v1 id=impl.scc-graph-recompile-scoped-owners work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn recompile_scoped_with_owners(
    store: &Store,
    changed_files: &[String],
    known_owners: &[String],
    component_signals: Vec<components::ComponentSignal>,
) -> Result<(RecompileReport, AffectedClosure)> {
    let graph = RealityGraph::load(store)?;
    let closure = affected_closure_with_owners(&graph, changed_files, known_owners);
    let report = CompilationPipeline::new(store)
        .component_signals(component_signals)
        .run()?;
    Ok((report, closure))
}

/// Recompile the entire derived layer (components, flows, invariants, drift)
/// from the reality graph. Idempotent; replaces derived tables in the store.
/// Equivalent to [`CompilationPipeline::run`] (kept for callers that do not
/// need stage control).
pub fn recompile(store: &Store) -> Result<RecompileReport> {
    CompilationPipeline::new(store).run()
}

#[derive(Debug, Clone, Default)]
pub struct RecompileReport {
    pub components: usize,
    pub flows: usize,
    pub invariants: usize,
    /// Number of drift findings emitted at stage 5 (against the freshly
    /// compiled model).
    pub drift: usize,
    /// Number of trust-boundary crossing edges at stage 6.
    pub boundaries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_empty_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = RealityGraph::load(&store).unwrap();
        assert!(g.entities.is_empty());
        let rep = recompile(&store).unwrap();
        assert_eq!(rep.components, 1, "empty repos still get a root component");
    }

    #[test]
    fn pipeline_bumps_derived_epoch_and_reloads_between_stages() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        let epoch_before = store.model_epoch().unwrap();
        let rep = CompilationPipeline::new(&store).run().unwrap();
        let epoch_after = store.model_epoch().unwrap();

        // derived compilation invalidates the cache epoch even on an empty
        // store (fail closed before any derived write)
        assert_eq!(epoch_after.derived, epoch_before.derived + 1);
        assert!(rep.components >= 1);
        assert_eq!(rep.drift, 0, "no drift on an empty model");

        // a second run is idempotent in content but bumps again (each
        // recompile is a new derived model state)
        let rep2 = CompilationPipeline::new(&store).run().unwrap();
        assert_eq!(rep2.components, rep.components);
        assert_eq!(rep2.flows, rep.flows);
        assert_eq!(rep2.invariants, rep.invariants);
    }

    #[test]
    fn drift_uses_newly_compiled_flows() {
        // regression (P0 stage ordering): drift findings must be computed
        // against flows written by the current pipeline run, not a stale
        // pre-recompile model.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();

        // first compile: only the empty-repo architecture flow exists
        let rep = CompilationPipeline::new(&store).run().unwrap();
        let base_flows = rep.flows;
        assert!(base_flows >= 1);

        // add a flow-affecting fact: a route handler
        let repo = store.repo_id.clone();
        let route = scc_core::entity_id(&repo, "route", "get-/api/x");
        store
            .insert_entity(
                scc_core::Entity::new(route.clone(), "route", "get-/api/x")
                    .attr("method", serde_json::json!("GET"))
                    .attr("path", serde_json::json!("/api/x"))
                    .attr("handler", serde_json::json!("handle_x")),
                &["main.py".into()],
            )
            .unwrap();
        let sym = scc_core::symbol_id(&repo, "main.py", "handle_x");
        store
            .insert_entity(&scc_core::Entity::new(sym.clone(), "symbol", "handle_x"), &["main.py".into()])
            .unwrap();
        store
            .insert_relationship(
                &scc_core::Relationship::new(
                    "rel:route",
                    sym.clone(),
                    scc_core::predicates::HANDLES,
                    route,
                    scc_core::Provenance::Extracted,
                ),
                "main.py",
            )
            .unwrap();

        // second compile: the pipeline must see its own newly written
        // component (root) and flow (get-/api/x) in later stages
        let rep2 = CompilationPipeline::new(&store).run().unwrap();
        assert!(rep2.flows > base_flows, "flows: {} vs {base_flows}", rep2.flows);
        let flows = store.flows().unwrap();
        assert!(
            flows.iter().any(|f| f.name.contains("get-/api/x")),
            "compiled flow must be present: {flows:?}"
        );
    }

    // trace:exempt reason=test-helper  # hermetic git fixture factory (test-only)
    fn git_init(dir: &std::path::Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "SCC Test"],
            // Hermetic: a user's global commit.gpgsign=true must not leak
            // into test repos (gpg-agent exhaustion under parallel load
            // made commits flaky).
            vec!["config", "commit.gpgsign", "false"],
        ] {
            let out = std::process::Command::new("git")
                .args(&args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        }
    }

    fn git_commit_all(dir: &std::path::Path, msg: &str) {
        let out = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success());
        let out = std::process::Command::new("git")
            .args(["commit", "-q", "-m", msg])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git commit failed");
    }

    fn git_write(dir: &std::path::Path, name: &str, content: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn pipeline_wires_cochange_into_clustering() {
        // Wave 5: the pipeline computes git co-change pairs before stage 1,
        // feeds them into the clustering score (+2 per pair fully inside a
        // candidate), and annotates the freshly written components via
        // cochange::enrich_components — all in one run.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git_init(&root);
        git_write(&root, "src/a.py", "a = 1\n");
        git_write(&root, "src/b.py", "b = 2\n");
        git_commit_all(&root, "c1");
        git_write(&root, "src/a.py", "a = 2\n");
        git_write(&root, "src/b.py", "b = 3\n");
        git_commit_all(&root, "c2");

        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        for f in ["src/a.py", "src/b.py"] {
            let id = scc_core::entity_id(&store.repo_id, scc_core::kinds::FILE, f);
            store
                .insert_entity(
                    &scc_core::Entity::new(id, scc_core::kinds::FILE, f),
                    &[f.into()],
                )
                .unwrap();
        }
        store
            .replace_intent_claims(&[(
                "component".to_string(),
                serde_json::json!({"name": "core", "paths": ["src"]}),
            )])
            .unwrap();

        let rep = CompilationPipeline::new(&store).run().unwrap();
        assert!(rep.components >= 2, "root + core: {}", rep.components);

        let comps = store.components().unwrap();
        let core = comps.iter().find(|c| c.name == "core").unwrap();
        assert_eq!(
            core.attributes["boundary_kind"],
            serde_json::json!("declared")
        );
        // one pair (src/a.py <-> src/b.py) fully inside "src": +2.0
        assert_eq!(
            core.attributes["clustering_score"],
            serde_json::json!(2.0),
            "{:?}",
            core.attributes
        );
        // enrich_components ran on the freshly written components
        let cc = core.attributes["cochange"].clone();
        assert_eq!(cc["top"], 2);
        assert!(
            cc["pairs"][0]
                .as_str()
                .unwrap()
                .starts_with("src/a.py <-> src/b.py"),
            "{cc}"
        );
        // a second identical run is deterministic
        let rep2 = CompilationPipeline::new(&store).run().unwrap();
        assert_eq!(rep2.components, rep.components);
        let comps2 = store.components().unwrap();
        let core2 = comps2.iter().find(|c| c.name == "core").unwrap();
        assert_eq!(
            core.attributes["clustering_score"],
            core2.attributes["clustering_score"]
        );
    }
}
