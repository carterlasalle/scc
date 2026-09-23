//! Impact analysis (docs/API_AND_INTEGRATIONS.md §2 `impact_context`).
//!
//! Given files/symbols/diff, determine: affected components, flows,
//! upstream/downstream consumers, contracts (routes), data, invariants,
//! tests, and a risk assessment.

use crate::components::component_for_path;
use crate::{trust::TrustedGraphView, Result};
use scc_core::kinds;
use scc_core::Severity;
use scc_store::Store;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

#[derive(Debug, Clone, Default, serde::Serialize)]
// trace:exempt reason=internal-detail
pub struct Impact {
    pub files: Vec<String>,
    pub components: Vec<String>, // component ids
    pub flows: Vec<String>,      // flow ids
    pub upstream: Vec<String>,   // component ids that depend on affected
    pub downstream: Vec<String>, // component ids affected depends on
    pub contracts: Vec<String>,  // route ids
    pub data: Vec<String>,       // store/data entity ids
    pub invariants: Vec<String>, // invariant ids
    pub tests: Vec<String>,      // test entity ids
    pub risk: String,            // low | medium | high
    #[serde(default)]
    pub notes: Vec<String>,
    /// Per-file importer closure: (importing file, depth, provenance).
    /// The primary impact signal — components/flows interpret it, and the
    /// pack renders it first so a glued component never hides file truth.
    #[serde(default)]
    pub importers: Vec<Importer>,
    /// Historical co-change partners not in the current file set.
    /// Never merged into `components` / `flows` / `contracts` / `data`.
    #[serde(default)]
    pub forgotten_partners: Vec<ForgottenPartner>,
}

/// A file that historically changes with an affected file but is not in
/// the current impact file set. Reason is always `cochange` here.
/// A file importing (transitively) an impact target, with BFS depth and
/// the provenance of the edge that discovered it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
// trace:exempt reason=internal-detail
pub struct Importer {
    pub file: String,
    pub depth: u32,
    pub provenance: scc_core::Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
// trace:exempt reason=internal-detail
pub struct ForgottenPartner {
    pub file: String,
    pub partner: String,
    pub commits: u32,
    pub reason: String,
}

/// Co-change partners of `changed` that are not themselves in `changed`.
/// Deterministic: sorted by commits desc, then file, then partner.
// trace:v1 id=impl.scc.impact.forgotten-partners work=WORK-ripwire-lessons-phase6 satisfies=REQ-forgotten-impact-partners
pub fn forgotten_cochange_partners(
    pairs: &[crate::cochange::CochangePair],
    changed: &[String],
) -> Vec<ForgottenPartner> {
    let changed_set: std::collections::BTreeSet<&str> =
        changed.iter().map(|s| s.as_str()).collect();
    let mut out: Vec<ForgottenPartner> = Vec::new();
    for pair in pairs {
        let (file, partner) = if changed_set.contains(pair.a.as_str())
            && !changed_set.contains(pair.b.as_str())
        {
            (pair.a.clone(), pair.b.clone())
        } else if changed_set.contains(pair.b.as_str()) && !changed_set.contains(pair.a.as_str()) {
            (pair.b.clone(), pair.a.clone())
        } else {
            continue;
        };
        out.push(ForgottenPartner {
            file,
            partner,
            commits: pair.commits,
            reason: "cochange".into(),
        });
    }
    out.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.partner.cmp(&b.partner))
    });
    out
}

/// Second wave — same-package / re-export callers: a file whose symbol
/// calls a symbol defined in an affected file is itself affected, even with
/// no import edge (Go same-package calls, unresolved receivers, facade
/// re-exports). Depth-graded like the import wave: seed depth + 1,
/// transitively.
#[allow(clippy::too_many_arguments)]
// trace:v1 id=impl.scc.impact.caller-wave work=WORK-SI-MMMJA4G6 satisfies=REQ-SCC-IR
fn caller_wave(
    view: &TrustedGraphView,
    graph: &crate::RealityGraph,
    importer_depth: &mut BTreeMap<String, u32>,
    importer_prov: &mut BTreeMap<String, scc_core::Provenance>,
    max_depth: u32,
) {
    let mut queue: VecDeque<(String, u32)> = importer_depth.iter().map(|(f, d)| (f.clone(), *d)).collect();
        // callee symbol id -> defining file (sweep once, not per edge).
        let mut sym_file: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for e in graph.entities_of_kind(kinds::SYMBOL) {
            if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
                sym_file.insert(e.id.as_str(), f);
            }
        }
        while let Some((path, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let target_file_id = scc_core::entity_id(&graph.repo_id, kinds::FILE, &path);
            // symbols defined in this file: CONTAINS edges file -> symbol.
            let mut owned: Vec<&str> = Vec::new();
            for r in view.out_pred(&target_file_id, scc_core::predicates::CONTAINS) {
                owned.push(r.object.as_str());
            }
            // reverse call edges into those symbols.
            let mut callers: Vec<(&str, scc_core::Provenance)> = Vec::new();
            for sym in &owned {
                for r in view.in_pred(sym, scc_core::predicates::CALLS) {
                    callers.push((r.subject.as_str(), r.provenance));
                }
            }
            callers.sort_by(|a, b| a.0.cmp(b.0));
            callers.dedup_by(|a, b| a.0 == b.0);
            for (caller_id, prov) in callers {
                let Some(caller_file) = sym_file.get(caller_id).copied() else { continue };
                if caller_file == path {
                    continue;
                }
                match importer_depth.get(caller_file) {
                    Some(&d) if d <= depth + 1 => {}
                    _ => {
                        importer_depth.insert(caller_file.to_string(), depth + 1);
                        importer_prov.entry(caller_file.to_string()).or_insert(prov);
                        queue.push_back((caller_file.to_string(), depth + 1));
                    }
                }
            }
        }
    }

// trace:v1 id=impl.scc.impact work=WORK-SCC-013 satisfies=REQ-forgotten-impact-partners,REQ-SCC-IR
// trace:v1 id=impl.scc.impact.importers work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
// trace:v1 id=impl.scc.impact.flowexact work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn compute_impact(
    view: &TrustedGraphView,
    store: &Store,
    files: &[String],
    symbols: &[String],
) -> Result<Impact> {
    let graph = &view.graph;
    let mut imp = Impact::default();

    let file_ids: HashSet<String> = files
        .iter()
        .map(|f| scc_core::entity_id(&graph.repo_id, kinds::FILE, f))
        .collect();
    // Ghost inputs must never produce a confident report: partition the
    // requested files into indexed vs unknown. Unknown targets are reported
    // in notes (partial) or refuse the whole query (all unknown).
    let (resolved_files, unresolved_files): (Vec<&String>, Vec<&String>) =
        files.iter().partition(|f| {
            view.entity(&scc_core::entity_id(&graph.repo_id, kinds::FILE, f)).is_some()
        });
    let sym_ids: HashSet<String> = symbols
        .iter()
        .map(|s| scc_core::symbol_id(&graph.repo_id, "?", s))
        .collect();
    // symbols may be given as plain names — resolve against the index
    // (one full symbol sweep, not one per requested symbol).
    let mut resolved_sym_ids: HashSet<String> = HashSet::new();
    let all_symbols = graph.entities_of_kind(kinds::SYMBOL);
    for s in symbols {
        let matches: Vec<String> = all_symbols
            .iter()
            .filter(|e| e.name == *s)
            .map(|e| e.id.clone())
            .collect();
        if matches.is_empty() {
            // exact entity id?
            if view.entity(s).is_some() {
                resolved_sym_ids.insert(s.clone());
            }
        } else {
            resolved_sym_ids.extend(matches);
        }
    }
    for id in &sym_ids {
        // symbol_id with "?" file is a miss; resolved ones are real
        if !id.ends_with("/?/") && view.entity(id).is_some() {
            resolved_sym_ids.insert(id.clone());
        }
    }
    // File-scoped symbols: index symbols by file ONCE, then take only the
    // requested files' symbols. The old code swept all symbols per query;
    // scoping keeps this linear in the request, not the repo.
    {
        let mut by_file: HashSet<String> = HashSet::new();
        for f in &resolved_files {
            by_file.insert((*f).clone());
        }
        for e in &all_symbols {
            if let Some(f) = e.attributes.get("file").and_then(|v| v.as_str()) {
                if by_file.contains(f) {
                    resolved_sym_ids.insert(e.id.clone());
                }
            }
        }
    }

    // Cochange history is a legitimate signal on paths without file entities
    // (an unindexed repo still has git history): only refuse when there is
    // genuinely nothing to analyze — no resolved graph targets AND no
    // cochange pair touching a requested file. Otherwise proceed with a note.
    let pairs = crate::cochange::cached_cochange_pairs(store).unwrap_or_default();
    let has_cochange = files.iter().any(|f| {
        pairs.iter().any(|p| p.a == **f || p.b == **f)
    });
    if !files.is_empty() && resolved_files.is_empty() && resolved_sym_ids.is_empty() && !has_cochange {
        let mut unknown: Vec<String> = unresolved_files.iter().map(|s| s.to_string()).collect();
        unknown.extend(symbols.iter().filter(|s| {
            !resolved_sym_ids.iter().any(|r| r == *s || r.ends_with(&format!("/{s}")))
        }).cloned());
        unknown.sort();
        unknown.dedup();
        return Err(crate::GraphError::Impact(format!(
            "unknown target(s) {} — not in the index; refusing to fabricate",
            unknown.join(", ")
        )));
    }
    for f in &unresolved_files {
        imp.notes.push(format!("unknown target '{f}': not in index, excluded from analysis"));
    }

    // Per-file importer closure (the PRIMARY impact signal): BFS over
    // `imports` edges reversed (importer -> imported), seeded from the
    // resolved files. Components/flows below are interpretations of this
    // closure, not the closure itself — so a glued mega-component can make
    // them soupy without corrupting the file answer. Bounded: visited-set
    // dedup makes each file expand once; depth caps the fan-out. Provenance
    // travels with the edge; the shallowest depth wins on re-visit.
    let mut importer_depth: BTreeMap<String, u32> = BTreeMap::new();
    let mut importer_prov: BTreeMap<String, scc_core::Provenance> = BTreeMap::new();
    // trace:exempt reason=const-data
    const IMPORTER_MAX_DEPTH: u32 = 8;
    {
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        for f in &resolved_files {
            importer_depth.insert((*f).clone(), 0);
            queue.push_back(((*f).clone(), 0));
        }
        while let Some((path, depth)) = queue.pop_front() {
            if depth >= IMPORTER_MAX_DEPTH {
                continue;
            }
            let target_id = scc_core::entity_id(&graph.repo_id, kinds::FILE, &path);
            for r in view.in_pred(&target_id, scc_core::predicates::IMPORTS) {
                let importer = match view.entity(&r.subject) {
                    Some(e) if e.kind == kinds::FILE => e.name.clone(),
                    _ => continue,
                };
                if files.iter().any(|f| f == &importer) {
                    continue;
                }
                match importer_depth.get(&importer) {
                    Some(&d) if d <= depth + 1 => {}
                    _ => {
                        importer_depth.insert(importer.clone(), depth + 1);
                        importer_prov.insert(importer.clone(), r.provenance);
                        queue.push_back((importer, depth + 1));
                    }
                }
            }
        }
    }

    caller_wave(view, graph, &mut importer_depth, &mut importer_prov, IMPORTER_MAX_DEPTH);

    for (file, depth) in &importer_depth {
        if *depth == 0 {
            continue;
        }
        imp.importers.push(Importer {
            file: file.clone(),
            depth: *depth,
            provenance: importer_prov.get(file).copied().unwrap_or(scc_core::Provenance::Extracted),
        });
    }
    imp.importers.sort_by(|a, b| {
        a.depth.cmp(&b.depth).then_with(|| a.file.cmp(&b.file))
    });

    // affected components: components containing affected files or symbols
    let mut affected_comps: BTreeSet<String> = BTreeSet::new();
    let comps = store.components()?;
    for c in &comps {
        let paths: Vec<String> = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("paths"))
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let symbols_list: Vec<String> = c
            .attributes
            .get("implementation")
            .and_then(|i| i.get("symbols"))
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        for f in &resolved_files {
            let seg = component_for_path(f, &component_candidates(&comps));
            if seg == c.name {
                affected_comps.insert(c.id.clone());
            }
        }
        if !resolved_sym_ids.is_empty() {
            // resolved name-set built once per component scan (not once per
            // symbol): the inner entities_of_kind sweep was O(comps × syms).
            let sym_names: HashSet<String> = all_symbols
                .iter()
                .filter(|e| resolved_sym_ids.contains(&e.id))
                .map(|e| e.name.clone())
                .collect();
            for s in &symbols_list {
                if sym_names.contains(s) {
                    affected_comps.insert(c.id.clone());
                    break;
                }
            }
        }
        let _ = paths;
    }
    // also via contains relationships
    for c in &comps {
        for r in view.out_pred(&c.id, scc_core::predicates::CONTAINS) {
            if file_ids.contains(&r.object) {
                affected_comps.insert(c.id.clone());
            }
        }
    }

    imp.components = affected_comps.iter().cloned().collect();

    // flows containing affected components or their symbols
    let mut affected_syms: HashSet<String> = HashSet::new();
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::CONTAINS) {
            // file ids, expand to symbols
            for sr in view.out_pred(&r.object, scc_core::predicates::CONTAINS) {
                affected_syms.insert(sr.object.clone());
            }
        }
    }
    affected_syms.extend(resolved_sym_ids.iter().cloned());

    // Flow matching by exact step identity — never substring. Steps carry
    // (actor, operation) over component/symbol ids; the old code ran a
    // steps×(components+symbols+files) `contains` matrix, which is both
    // quadratic AND wrong (component "api" matches every actor containing
    // those letters; django never finished). Exact id membership is linear
    // in total steps and terminates by construction — no time budget needed.
    let mut seen_flows: HashSet<&str> = HashSet::new();
    let all_flows = view.flows();
    // affected files' ids + resolved symbol ids: the exact step vocabulary.
    // (owned Strings — file_ids/affected_comps outlive this block.)
    let mut step_vocab: HashSet<&str> = HashSet::new();
    for id in &file_ids {
        step_vocab.insert(id.as_str());
    }
    for sid in &resolved_sym_ids {
        step_vocab.insert(sid.as_str());
    }
    for cid in &affected_comps {
        step_vocab.insert(cid.as_str());
    }
    // requested file names too (steps sometimes name the path, not the id).
    for f in files {
        step_vocab.insert(f.as_str());
    }
    for flow in &all_flows {
        let steps_mention = flow.steps.iter().any(|s| {
            step_vocab.contains(s.actor.as_str()) || step_vocab.contains(s.operation.as_str())
        });
        if steps_mention && seen_flows.insert(flow.id.as_str()) {
            imp.flows.push(flow.id.clone());
        }
    }
    // entrypoint attribute on flows
    for flow in &all_flows {
        if let Some(ep) = flow.attributes.get("entrypoint").and_then(|v| v.as_str()) {
            if affected_syms.contains(ep) && seen_flows.insert(flow.id.as_str()) {
                imp.flows.push(flow.id.clone());
            }
        }
    }

    // upstream (depend on affected) / downstream (affected depends on)
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::DEPENDS_ON) {
            imp.downstream.push(r.object.clone());
        }
        for r in view.in_pred(cid, scc_core::predicates::DEPENDS_ON) {
            imp.upstream.push(r.subject.clone());
        }
    }
    imp.upstream.sort();
    imp.upstream.dedup();
    imp.downstream.sort();
    imp.downstream.dedup();

    // contracts: routes handled by affected symbols
    for sid in &affected_syms {
        for r in view.out_pred(sid, scc_core::predicates::HANDLES) {
            imp.contracts.push(r.object.clone());
        }
    }

    // data: stores owned by affected components + accessed by affected symbols
    for cid in &affected_comps {
        for r in view.out_pred(cid, scc_core::predicates::OWNS) {
            imp.data.push(r.object.clone());
        }
    }
    for sid in &affected_syms {
        for pred in ["reads", "writes", "queries"] {
            for r in view.out_pred(sid, pred) {
                imp.data.push(r.object.clone());
            }
        }
    }
    imp.data.sort();
    imp.data.dedup();

    // invariants whose scope intersects affected entities
    for inv in &view.invariants() {
        let scoped = inv
            .scope
            .iter()
            .any(|s| affected_comps.contains(s) || imp.data.contains(s));
        if scoped {
            imp.invariants.push(inv.id.clone());
        }
    }

    // tests covering affected symbols
    for sid in &affected_syms {
        for r in view.out_pred(sid, scc_core::predicates::TESTED_BY) {
            imp.tests.push(r.object.clone());
        }
    }
    imp.tests.sort();
    imp.tests.dedup();

    imp.files = files.to_vec();

    // risk: high if critical invariants affected or contracts changed;
    // medium if flows affected; else low
    let critical_invariants = imp
        .invariants
        .iter()
        .filter(|iid| {
            graph
                .invariants
                .iter()
                .find(|i| i.id == **iid)
                .map(|i| i.severity == Severity::Critical)
                .unwrap_or(false)
        })
        .count();
    if critical_invariants > 0 || !imp.contracts.is_empty() {
        imp.risk = "high".into();
        if critical_invariants > 0 {
            imp.notes.push(format!(
                "{critical_invariants} critical invariant(s) in scope of the change"
            ));
        }
        if !imp.contracts.is_empty() {
            imp.notes.push(format!(
                "{} API contract(s) (routes) affected — consumers may break",
                imp.contracts.len()
            ));
        }
    } else if !imp.flows.is_empty() || !imp.tests.is_empty() {
        imp.risk = "medium".into();
    } else {
        imp.risk = "low".into();
    }
    if !imp.tests.is_empty() {
        imp.notes.push(format!(
            "{} test(s) exercise the affected code",
            imp.tests.len()
        ));
    }

    let pairs = crate::cochange::cached_cochange_pairs(store).unwrap_or_default();
    imp.forgotten_partners = forgotten_cochange_partners(&pairs, &imp.files);
    if !imp.forgotten_partners.is_empty() {
        imp.notes.push(format!(
            "{} forgotten co-change partner(s) — historical, not EXTRACTED impact",
            imp.forgotten_partners.len()
        ));
    }

    Ok(imp)
}

// trace:exempt reason=internal-detail
fn component_candidates(comps: &[scc_core::Entity]) -> Vec<crate::components::ComponentCandidate> {
    comps
        .iter()
        .map(|c| {
            let mut dirs: Vec<String> = c
                .attributes
                .get("implementation")
                .and_then(|i| i.get("paths"))
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if dirs.is_empty() {
                dirs.push(c.name.clone());
            }
            crate::components::ComponentCandidate {
                name: c.name.clone(),
                dirs,
                boundary_kind: c
                    .attributes
                    .get("boundary_kind")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| crate::components::BOUNDARY_CODE_REGION.to_string()),
                intent: c
                    .attributes
                    .get("intent")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            }
        })
        .collect()
}

/// Files/symbols in the current diff (git diff --name-only).
// trace:exempt reason=internal-detail
pub fn diff_files(store: &Store, base: Option<&str>) -> Result<Vec<String>> {
    let root = &store.root;
    let mut cmd = std::process::Command::new("git");
    cmd.args(["diff", "--name-only", "--diff-filter=ACMRT"]);
    if let Some(b) = base {
        cmd.arg(format!("{b}...HEAD"));
    } else {
        cmd.arg("HEAD");
    }
    cmd.arg("--");
    let out = cmd
        .current_dir(root)
        .output()
        .map_err(|e| scc_store::StoreError::NotInitialized(format!("git diff failed: {e}")))?;
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim();
        if !line.is_empty() {
            files.push(line.to_string());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    // trace:exempt reason=unit-test
    fn fixture_two_file_call() -> (tempfile::TempDir, Store) {
        use scc_core::{Entity, Relationship};
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let repo = store.repository().id.clone();
        // a.go defines Svc.Do; b.go calls it with no import edge
        // (same-package shape).
        for (path, sym) in [("a.go", "Svc.Do"), ("b.go", "main")] {
            let fid = scc_core::entity_id(&repo, kinds::FILE, path);
            store.insert_entity(&Entity::new(fid.clone(), kinds::FILE, path.to_string()), &[path.to_string()]).unwrap();
            let sid = scc_core::entity_id(&repo, kinds::SYMBOL, &format!("{path}/{sym}"));
            let mut se = Entity::new(sid.clone(), kinds::SYMBOL, sym.to_string());
            se.attr("file", serde_json::json!(path));
            store.insert_entity(&se, &[path.to_string()]).unwrap();
            store.insert_relationship(&Relationship::new(
                format!("contains-{path}"), fid, scc_core::predicates::CONTAINS, sid, scc_core::Provenance::Extracted,
            ), path).unwrap();
        }
        let a_sym = scc_core::entity_id(&repo, kinds::SYMBOL, "a.go/Svc.Do");
        let b_sym = scc_core::entity_id(&repo, kinds::SYMBOL, "b.go/main");
        store.insert_relationship(&Relationship::new(
            "calls-b-a".to_string(), b_sym, scc_core::predicates::CALLS, a_sym, scc_core::Provenance::Extracted,
        ), "b.go").unwrap();
        (dir, store)
    }

    #[test]
    // trace:v1 id=test.scc.impact.caller-wave verifies=REQ-SCC-IR exercises=impl.scc.impact.caller-wave
    fn caller_wave_pulls_same_package_callers() {
        let (_dir, store) = fixture_two_file_call();
        let g = crate::RealityGraph::load(&store).unwrap();
        let v = TrustedGraphView::new(&g, &store, &[], crate::TrustPolicy::default());
        let imp = compute_impact(&v, &store, &["a.go".to_string()], &[]).unwrap();
        assert!(imp.importers.iter().any(|i| i.file == "b.go"), "{imp:?}");
    }

    #[test]
    // trace:exempt reason=internal-detail
    fn empty_impact() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = crate::RealityGraph::load(&store).unwrap();
        let v = TrustedGraphView::new(&g, &store, &[], crate::TrustPolicy::default());
        let imp = compute_impact(&v, &store, &[], &[]).unwrap();
        assert!(imp.components.is_empty());
        assert_eq!(imp.risk, "low");
        assert!(imp.forgotten_partners.is_empty());
    }

    #[test]
    // trace:v1 id=test.scc.impact.ghost-target-refused verifies=REQ-SI-503JSBGP exercises=impl.scc.impact
    fn ghost_target_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        let g = crate::RealityGraph::load(&store).unwrap();
        let v = TrustedGraphView::new(&g, &store, &[], crate::TrustPolicy::default());
        let err = compute_impact(&v, &store, &["src/handle.rs".into()], &[])
            .expect_err("nonexistent path must not produce a report");
        assert!(err.to_string().contains("refusing to fabricate"), "{err}");
        assert!(err.to_string().contains("src/handle.rs"), "{err}");
    }

    #[test]
    // trace:v1 id=test.scc.impact.forgotten-partners verifies=REQ-forgotten-impact-partners exercises=impl.scc.impact.forgotten-partners
    fn forgotten_partners_are_not_semantic_impact() {
        let pairs = vec![
            crate::cochange::CochangePair {
                a: "src/a.py".into(),
                b: "src/b.py".into(),
                commits: 3,
            },
            crate::cochange::CochangePair {
                a: "src/a.py".into(),
                b: "src/c.py".into(),
                commits: 2,
            },
        ];
        let found = forgotten_cochange_partners(&pairs, &["src/a.py".into()]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].partner, "src/b.py");
        assert_eq!(found[0].commits, 3);
        assert_eq!(found[0].reason, "cochange");
        assert_eq!(found[1].partner, "src/c.py");
        let none = forgotten_cochange_partners(
            &pairs,
            &["src/a.py".into(), "src/b.py".into(), "src/c.py".into()],
        );
        assert!(
            none.is_empty(),
            "partners already in the change set are not forgotten"
        );
    }
}
