//! Engine ranking namespace: every ranker stage callable independently.
//!
//! Thin wrappers over the existing `scc-context` pipeline pieces — no
//! algorithm changes. `ranking.symbols` runs the FULL blend (same math as
//! `build_surface`) and returns per-item feature decomposition plus
//! plugin contributions; stage ops expose the raw vectors.
//! Plugin hooks (spec 18): seed providers, edge-weight contributors,
//! rank features, and rerankers chain deterministically; every
//! contribution is recorded in the explanation.

use scc_api::{RankFeatures, RankItem, RankRequest, RankResult};

// trace:exempt reason=internal-detail
pub struct Ranker<'a> {
    engine: &'a crate::workspace::Engine<'a>,
}

// trace:exempt reason=internal-detail
impl<'a> Ranker<'a> {
    // trace:exempt reason=internal-detail
    pub fn new(engine: &'a crate::workspace::Engine<'a>) -> Self { Ranker { engine } }

    // trace:exempt reason=internal-detail
    fn ctx(&self) -> scc_context::ContextCompiler<'_> { self.engine.ctx() }

    /// Raw global PageRank vector: (node_id, score) over the full
    /// heterogeneous universe, id-sorted. No projection, no blending.
    // trace:exempt reason=internal-detail
    pub fn pagerank_global(&self) -> crate::Result<Vec<(String, f64)>> {
        self.pagerank_global_with(&[])
    }

    /// Global vector with edge-weight contributors applied.
    // trace:exempt reason=internal-detail
    pub fn pagerank_global_with(
        &self,
        contributors: &[EdgeWeightFn],
    ) -> crate::Result<Vec<(String, f64)>> {
        self.pagerank_global_with_hooks(contributors, &[])
    }

    /// Global vector with edge weights and extra rank-time edges applied.
    // trace:exempt reason=internal-detail
    pub fn pagerank_global_with_hooks(
        &self,
        contributors: &[EdgeWeightFn],
        extra: &[(String, String, String, f64)],
    ) -> crate::Result<Vec<(String, f64)>> {
        let ctx = self.ctx();
        let owned: Vec<EdgeWeightFn> = contributors.iter().cloned().collect();
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust_and_extra(
            &ctx.view,
            move |s, p, o, b| Self::fold_edge_contributors(&owned, s, p, o, b).0,
            extra,
        );
        let v = ranker.global_vector();
        Ok(ranker.nodes().iter().cloned().zip(v).collect())
    }

    /// Normalized reference graph (§123 intermediate): trusted
    /// relationships mapped to reference kinds (call/read/write/…).
    /// Read-only projection — the reference surface `SystemRanker`
    /// diffuses over, with per-edge provenance and confidence.
    // trace:exempt reason=internal-detail
    pub fn reference_graph(&self) -> crate::Result<Vec<scc_core::ReferenceEdge>> {
        let ctx = self.ctx();
        Ok(scc_context::pagerank::build_reference_graph(&ctx.view))
    }

    /// Rank-universe nodes (§123 intermediate): (id, kind) over the
    /// full heterogeneous universe in rank order. Same nodes every
    /// vector is indexed by — the node table for pagerank.global/task.
    // trace:exempt reason=internal-detail
    pub fn universe(&self) -> crate::Result<Vec<(String, String)>> {
        self.universe_with(&RankHooks::default(), "")
    }

    /// Rank universe merged with plugin rank-node providers (§124 item
    /// 17): the same table `symbols_with_hooks` diffuses over. Read-only
    /// stage view — no entity, relationship, or evidence is written.
    // trace:exempt reason=internal-detail
    pub fn universe_with(&self, hooks: &RankHooks, goal: &str) -> crate::Result<Vec<(String, String)>> {
        let ctx = self.ctx();
        let mut extra_nodes: Vec<(String, String)> = Vec::new();
        for prov in &hooks.rank_nodes {
            extra_nodes.extend(prov(goal));
        }
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust_extra_and_nodes(
            &ctx.view,
            |_s: &str, _p: &str, _o: &str, _b: f64| None,
            &[],
            &extra_nodes,
        );
        Ok(ranker.nodes().iter().cloned().zip(ranker.kinds().iter().cloned()).collect())
    }

    /// Project a universe vector to per-symbol scores (§123
    /// intermediate): entity importance reaching owner/handler symbols.
    /// `vector` is (id, score) pairs over universe ids (e.g. a
    /// pagerank.global/task row); unknown ids score 0. Same projection
    /// `symbols_with_hooks` blends from — exposed for debuggability.
    // trace:exempt reason=internal-detail
    pub fn project_symbols(&self, vector: &[(String, f64)]) -> crate::Result<Vec<(String, f64)>> {
        let ctx = self.ctx();
        let ranker = scc_context::pagerank::SystemRanker::new(&ctx.view);
        let by_id: std::collections::BTreeMap<&str, f64> =
            vector.iter().map(|(id, s)| (id.as_str(), *s)).collect();
        let full: Vec<f64> = ranker.nodes().iter().map(|id| by_id.get(id.as_str()).copied().unwrap_or(0.0)).collect();
        Ok(ranker.project_to_symbols(&full))
    }

    /// Raw rank-universe edges (§123.12): (subject, predicate, object,
    /// base weight), pre-aggregation. No plugin hooks by contract — the
    /// structure the vectors diffuse over.
    // trace:exempt reason=internal-detail
    pub fn rank_edges(&self) -> crate::Result<Vec<(String, String, String, f64)>> {
        let ctx = self.ctx();
        let ranker = scc_context::pagerank::SystemRanker::new(&ctx.view);
        Ok(ranker.rank_edges())
    }

    /// Raw task-personalized PPR vector for goal.
    // trace:exempt reason=internal-detail
    pub fn pagerank_task(&self, goal: &str) -> crate::Result<Vec<(String, f64)>> {
        self.pagerank_task_with(goal, &[])
    }

    /// Task vector with edge-weight contributors applied.
    // trace:exempt reason=internal-detail
    pub fn pagerank_task_with(
        &self,
        goal: &str,
        contributors: &[EdgeWeightFn],
    ) -> crate::Result<Vec<(String, f64)>> {
        self.pagerank_task_with_hooks(goal, contributors, &[])
    }

    /// Task vector with edge weights and extra rank-time edges applied.
    // trace:exempt reason=internal-detail
    pub fn pagerank_task_with_hooks(
        &self,
        goal: &str,
        contributors: &[EdgeWeightFn],
        extra: &[(String, String, String, f64)],
    ) -> crate::Result<Vec<(String, f64)>> {
        let ctx = self.ctx();
        let owned: Vec<EdgeWeightFn> = contributors.iter().cloned().collect();
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust_and_extra(
            &ctx.view,
            move |s, p, o, b| Self::fold_edge_contributors(&owned, s, p, o, b).0,
            extra,
        );
        let seeds = lexical_seeds(&ctx, goal);
        let v = ranker.task_vector(&seeds);
        Ok(ranker.nodes().iter().cloned().zip(v).collect())
    }

    /// Fold chained edge-weight contributors over one base weight.
    /// Returns (adjustment for the ranker, applied modes for reasons).
    /// Unknown modes are no-change (never silent corruption).
    // trace:exempt reason=internal-detail
    fn fold_edge_contributors(
        contributors: &[EdgeWeightFn],
        subject: &str,
        predicate: &str,
        object: &str,
        base: f64,
    ) -> (Option<(String, f64)>, Vec<String>) {
        let mut acc: Option<(String, f64)> = None;
        let mut applied = Vec::new();
        for c in contributors {
            // Chain over the running value: re-resolve base through acc.
            let current = match &acc {
                None => base,
                Some((m, v)) => apply_edge_weight(base, m, *v),
            };
            if let Some((mode, value)) = c(subject, predicate, object, current) {
                match mode.as_str() {
                    "add" | "multiply" | "replace" | "veto" => {
                        applied.push(mode.clone());
                        acc = Some((mode, value));
                    }
                    _ => {}
                }
            }
        }
        (acc, applied)
    }

    /// Engine required-coverage base set (entry ids `build_surface`
    /// partitions on): invariant/invocation/flow/state-owner entries.
    /// The op unions plugin coverage providers over this.
    // trace:exempt reason=internal-detail
    pub fn required_with(&self, goal: &str, hooks: &RankHooks) -> crate::Result<std::collections::BTreeSet<String>> {
        let ctx = self.ctx();
        let map = scc_context::surface::compile_surface_map(&ctx);
        let mut required = scc_context::surface::required_ids(&map, &ctx);
        for cov in &hooks.coverage {
            for id in cov(goal) {
                required.insert(id);
            }
        }
        Ok(required)
    }

    /// Task-seed merge for goal (§123.11): lexical seeds + plugin
    /// providers, weights summed by id. Shared by `ranking.seeds` and
    /// `symbols_with_hooks` — one merge, two callers.
    // trace:exempt reason=internal-detail
    pub fn seeds_with(&self, goal: &str, hooks: &RankHooks) -> crate::Result<Vec<scc_core::TaskSeed>> {
        let ctx = self.ctx();
        let mut seeds = lexical_seeds(&ctx, goal);
        for seed_fn in &hooks.seed_providers {
            for x in seed_fn(goal) {
                if let Some(e) = seeds.iter_mut().find(|s| s.id == x.id) { e.weight += x.weight; }
                else { seeds.push(x); }
            }
        }
        seeds.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
        Ok(seeds)
    }

    /// Lexical candidate generation for goal (stage 1).
    // trace:exempt reason=internal-detail
    pub fn candidates(&self, goal: &str, limit: usize) -> crate::Result<Vec<scc_context::rank::ScoredEntity>> {
        self.candidates_with(goal, limit, &RankHooks::default())
    }

    /// Candidates with explicit plugin providers (one call, deterministic
    /// order). Provider rows merge by canonical id: max score wins, the
    /// provider reason is tagged `plugin:<id>` for explainability.
    // trace:exempt reason=internal-detail
    pub fn candidates_with(
        &self,
        goal: &str,
        limit: usize,
        hooks: &RankHooks,
    ) -> crate::Result<Vec<scc_context::rank::ScoredEntity>> {
        let ctx = self.ctx();
        let mut merged: std::collections::BTreeMap<String, scc_context::rank::ScoredEntity> =
            scc_context::rank::collect_lexical_candidates(ctx.store, &ctx.view, goal, &[], limit.max(1))
                .into_iter().map(|c| (c.id.clone(), c)).collect();
        for prov in &hooks.candidates {
            for mut c in prov(goal) {
                if c.id.is_empty() {
                    continue;
                }
                c.reason = if c.reason.is_empty() { "plugin-candidate".into() } else { c.reason };
                match merged.get(&c.id) {
                    Some(prev) if prev.score >= c.score => {}
                    _ => { merged.insert(c.id.clone(), c); }
                }
            }
        }
        let mut out: Vec<scc_context::rank::ScoredEntity> = merged.into_values().collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
        out.truncate(limit.max(1));
        Ok(out)
    }

    /// Full task/global blend per symbol with feature decomposition.
    /// Same math as build_surface (no MMR/quotas/budget — pure ranking).
    /// Plugin seed/feature/rerank hooks apply here and are recorded.
    // trace:v1 id=impl.scc-engine-ranking.symbols work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
    pub fn symbols(&self, req: &RankRequest) -> crate::Result<RankResult> {
        self.symbols_with_hooks(req, &RankHooks::default())
    }

    /// Full ranking trace (§19): the RankResult plus the inputs the
    /// blend consumed — seed ids and required entry ids. Same
    /// computation as symbols_with_hooks (no new math); the envelope
    /// makes the decision auditable without re-deriving inputs.
    // trace:exempt reason=internal-detail
    pub fn trace_with_hooks(&self, req: &RankRequest, hooks: &RankHooks) -> crate::Result<(RankResult, Vec<String>, Vec<String>)> {
        let goal = req.goal.as_deref().unwrap_or("");
        let out = self.symbols_with_hooks(req, hooks)?;
        let mut seeds: Vec<String> = self.seeds_with(goal, hooks)?.into_iter().map(|x| x.id).collect();
        seeds.sort();
        let mut required: Vec<String> = self.required_with(goal, hooks)?.into_iter().collect();
        required.sort();
        Ok((out, seeds, required))
    }

    /// symbols() with explicit plugin hooks (one call, deterministic order).
    ///
    /// Derivation shares the pipeline's inputs BY CONSTRUCTION: per-entry
    /// confidence, required set, seed membership, and lexical scores come
    /// from the compiled surface map and its helpers — the same values
    /// `build_surface` blends. Per-symbol totals take the max over that
    /// symbol's entries (overloads); blends are otherwise identical math.
    // trace:v1 id=impl.scc-engine-ranking.symbols-hooks work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
    pub fn symbols_with_hooks(&self, req: &RankRequest, hooks: &RankHooks) -> crate::Result<RankResult> {
        // Named blend profiles (spec 12 + DoD 25): `default` plus any
        // plugin-registered profile. Unknown names fail loudly — silently
        // running default math under a requested profile would lie.
        let profile_w: Option<BlendWeights> = match req.profile.as_deref() {
            None | Some("default") => None,
            Some(name) => Some(hooks.profiles.get(name).cloned().ok_or_else(|| {
                let mut avail: Vec<&str> = hooks.profiles.keys().map(|s| s.as_str()).collect();
                avail.insert(0, "default");
                crate::EngineError::Other(format!(
                    "unknown ranking profile '{name}' (available: {})",
                    avail.join(", ")
                ))
            })?),
        };
        let profile_name: Option<&str> = req.profile.as_deref().filter(|p| *p != "default");
        let ctx = self.ctx();
        let goal = req.goal.as_deref().unwrap_or("");
        let goal_terms = scc_context::rank::terms(goal);
        let mut seeds = lexical_seeds(&ctx, goal);
        for seed_fn in &hooks.seed_providers {
            for s in seed_fn(goal) {
                if let Some(e) = seeds.iter_mut().find(|x| x.id == s.id) { e.weight += s.weight; }
                else { seeds.push(s); }
            }
        }
        let seed_ids: std::collections::BTreeSet<&str> =
            seeds.iter().map(|s| s.id.as_str()).collect();
        let owned: Vec<EdgeWeightFn> = hooks.edge_weights.iter().cloned().collect();
        // Extra rank-time edges (§48): collected per request, merged into
        // diffusion, never into the canonical graph.
        let mut extra: Vec<(String, String, String, f64)> = Vec::new();
        for prov in &hooks.rank_edges {
            extra.extend(prov(goal));
        }
        let extra_count = extra.len();
        // Extra rank-universe nodes (§124 item 17): merged before edge
        // indexing so provider edges can attach to provider nodes.
        let mut extra_nodes: Vec<(String, String)> = Vec::new();
        for prov in &hooks.rank_nodes {
            extra_nodes.extend(prov(goal));
        }
        let node_count = extra_nodes.len();
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust_extra_and_nodes(
            &ctx.view,
            move |s, p, o, b| Self::fold_edge_contributors(&owned, s, p, o, b).0,
            &extra,
            &extra_nodes,
        );
        let global_of: std::collections::BTreeMap<String, f64> =
            ranker.project_to_symbols(&ranker.global_vector()).into_iter().collect();
        let task_of: std::collections::BTreeMap<String, f64> =
            ranker.project_to_symbols(&ranker.task_vector(&seeds)).into_iter().collect();
        let has_task = !goal.is_empty();
        let map = scc_context::surface::compile_surface_map(&ctx);
        let mut required = scc_context::surface::required_ids(&map, &ctx);
        // Plugin coverage rules (§124 item 26): union contributed ids.
        let mut required_by: usize = 0;
        for cov in &hooks.coverage {
            for id in cov(goal) {
                if required.insert(id) {
                    required_by += 1;
                }
            }
        }
        let mut best: std::collections::BTreeMap<&str, RankItem> = std::collections::BTreeMap::new();
        for e in &map.entries {
            let task_ppr = task_of.get(&e.symbol_id).copied().unwrap_or(0.0);
            let global_ppr = global_of.get(&e.symbol_id).copied().unwrap_or(0.0);
            let lexical = scc_context::surface::entry_lexical(e, &goal_terms);
            let confidence = e.confidence as f64;
            let default_criticality = if seed_ids.contains(e.symbol_id.as_str()) || required.contains(&e.id) { 1.0 } else { importance_file_score(&e.path) };
            let (criticality, criticality_src) = resolve_override(&hooks.criticality, &e.symbol_id, goal, default_criticality);
            let (novelty, novelty_src) = resolve_override(&hooks.novelty, &e.symbol_id, goal, 1.0);
            let default_risk = if !e.path.is_empty() && ctx.stale_paths.iter().any(|p| p == &e.path) { 1.0 } else { 0.0 };
            let (change_risk, risk_src) = resolve_override(&hooks.risk, &e.symbol_id, goal, default_risk);
            let (semantic, semantic_src) = resolve_override(&hooks.semantic, &e.symbol_id, goal, 0.0);
            let blend = match profile_w.as_ref() {
                None => scc_context::pagerank::final_importance(task_ppr, global_ppr, lexical, semantic, confidence, criticality, change_risk, 0.0, has_task),
                Some(w) => {
                    use scc_context::pagerank as pr;
                    let (tw, gw) = if has_task { (w.task_ppr.unwrap_or(pr::TASK_PPR_WEIGHT), w.global_ppr.unwrap_or(pr::GLOBAL_PPR_WEIGHT)) }
                        else { (w.task_ppr.unwrap_or(0.0), w.global_ppr.unwrap_or(pr::NO_TASK_GLOBAL_WEIGHT)) };
                    tw * task_ppr
                        + gw * global_ppr
                        + w.lexical.unwrap_or(pr::LEXICAL_WEIGHT) * lexical
                        + w.semantic.unwrap_or(pr::SEMANTIC_WEIGHT) * semantic
                        + w.confidence.unwrap_or(pr::CONFIDENCE_WEIGHT) * confidence
                        + w.criticality.unwrap_or(pr::CRITICALITY_WEIGHT) * criticality
                        + w.change_risk.unwrap_or(pr::CHANGE_RISK_WEIGHT) * change_risk
                        + w.novelty.unwrap_or(pr::NOVELTY_WEIGHT) * 0.0
                }
            };
            let scale = 1.0 / (1.0 - scc_context::pagerank::SEMANTIC_WEIGHT);
            let novelty_w = profile_w.as_ref().and_then(|w| w.novelty).unwrap_or(scc_context::pagerank::NOVELTY_WEIGHT);
            let total = blend * scale + novelty_w * novelty;
            let mut plugin_features = std::collections::BTreeMap::new();
            let mut reasons: Vec<String> = Vec::new();
            if seed_ids.contains(e.symbol_id.as_str()) { reasons.push("task-seed".into()); }
            if let Some(src) = criticality_src { reasons.push(format!("criticality:{src}")); }
            if let Some(src) = novelty_src { reasons.push(format!("novelty:{src}")); }
            if let Some(src) = risk_src { reasons.push(format!("risk:{src}")); }
            if let Some(src) = semantic_src { reasons.push(format!("semantic:{src}")); }
            if required_by > 0 && required.contains(&e.id) && !seed_ids.contains(e.symbol_id.as_str()) { reasons.push(format!("required-by:plugin({required_by})")); }
            let mut total = total;
            for feat in &hooks.features {
                let v = feat(&e.symbol_id, goal);
                plugin_features.insert(v.name.clone(), v.score);
                total += v.weight * v.score;
                if !v.reason.is_empty() { reasons.push(v.reason.clone()); }
            }
            if let Some(pname) = profile_name {
                reasons.push(format!("profile:{pname}"));
            }
            let specificity = if e.exported { 1.15 } else { 1.0 };
            let item = RankItem { id: e.symbol_id.clone(), rank: total, position: 0,
                features: RankFeatures { task_ppr, global_ppr, lexical, semantic,
                    confidence, criticality, change_risk, novelty },
                specificity, reasons, plugin_features };
            match best.get(&e.symbol_id.as_str()) {
                Some(prev) if prev.rank >= total => {}
                _ => { best.insert(&e.symbol_id, item); }
            }
        }
        let mut items: Vec<RankItem> = best.into_values().collect();
        if !hooks.edge_weights.is_empty() {
            for it in items.iter_mut() {
                it.reasons.push(format!("edge-weights({})", hooks.edge_weights.len()));
            }
        }
        if extra_count > 0 {
            for it in items.iter_mut() {
                it.reasons.push(format!("rank-edges({extra_count})"));
            }
        }
        if node_count > 0 {
            for it in items.iter_mut() {
                it.reasons.push(format!("rank-nodes({node_count})"));
            }
        }
        for r in &hooks.rerankers { r(&mut items, goal); }
        items.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
        items.truncate(req.limit.max(1));
        for (i, it) in items.iter_mut().enumerate() { it.position = i + 1; }
        let mut warnings = Vec::new();
        if !hooks.edge_weights.is_empty() {
            warnings.push(format!(
                "{} edge-weight contributor(s) applied to the rank graph",
                hooks.edge_weights.len()
            ));
        }
        if extra_count > 0 {
            warnings.push(format!(
                "{extra_count} extra rank-time edge(s) merged into diffusion (never canonical)"
            ));
        }
        if node_count > 0 {
            warnings.push(format!(
                "{node_count} extra rank-universe node(s) merged into diffusion (never canonical)"
            ));
        }
        Ok(RankResult { items, omitted_ids: Vec::new(), warnings })
    }
}

// trace:exempt reason=internal-detail
fn lexical_seeds(ctx: &scc_context::ContextCompiler<'_>, goal: &str) -> Vec<scc_core::TaskSeed> {
    if goal.is_empty() { return Vec::new(); }
    scc_context::rank::collect_lexical_candidates(ctx.store, &ctx.view, goal, &[], 16)
        .into_iter()
        .map(|c| scc_core::TaskSeed { kind: c.kind, id: c.id, weight: c.score })
        .collect()
}


// trace:exempt reason=internal-detail
fn importance_file_score(path: &str) -> f64 {
    // Mirrors scc-context file_importance (pub(crate) there).
    let base = path.rsplit('/').next().unwrap_or(path);
    const IMPORTANT: &[&str] = &[
        "package.json", "pnpm-workspace.yaml", "yarn.lock", "Cargo.toml",
        "Cargo.lock", "go.mod", "pyproject.toml", "setup.py", "setup.cfg",
        "requirements.txt", "pom.xml", "build.gradle", "build.gradle.kts",
        "settings.gradle", "settings.gradle.kts", "gradlew", "Makefile",
        "CMakeLists.txt", "mix.exs", "Gemfile", "composer.json",
        "Dockerfile", "docker-compose.yml", "docker-compose.yaml",
        "compose.yml", "compose.yaml", ".dockerignore",
        ".github/workflows/ci.yml", ".github/workflows/main.yml",
        ".gitlab-ci.yml", "Jenkinsfile", "azure-pipelines.yml",
        ".circleci/config.yml", "buildkite.yml",
        "main.py", "main.go", "main.ts", "index.ts", "index.js",
        "app.py", "server.py", "server.ts", "server.js", "cli.py",
        "cli.ts", "cli.go", "src/main.rs", "bin/main.rs", "app.js",
        "app.ts",
    ];
    if IMPORTANT.contains(&base) || IMPORTANT.contains(&path) || path.starts_with(".github/workflows/") { 0.5 }
    else { 0.0 }
}

// trace:exempt reason=internal-detail
pub struct RankFeatureValue {
    pub name: String,
    pub score: f64,
    pub weight: f64,
    pub reason: String,
}

// trace:exempt reason=internal-detail
pub type SeedProvider = Box<dyn Fn(&str) -> Vec<scc_core::TaskSeed> + Send + Sync>;
// trace:exempt reason=internal-detail
pub type CandidateProvider =
    Box<dyn Fn(&str) -> Vec<scc_context::rank::ScoredEntity> + Send + Sync>;
// trace:exempt reason=internal-detail
pub type RankFeatureFn = Box<dyn Fn(&str, &str) -> RankFeatureValue + Send + Sync>;
// trace:exempt reason=internal-detail
pub type RerankerFn = Box<dyn Fn(&mut Vec<scc_api::RankItem>, &str) + Send + Sync>;
/// Edge-weight contributor: per-edge (subject, predicate, object, base)
/// adjustment. Return `Some((mode, value))` to alter the weight, `None`
/// for no change. Modes: add | multiply | replace | veto. Every applied
/// contribution is recorded on the affected rank items' reasons.
// trace:exempt reason=internal-detail
pub type EdgeWeightFn = std::sync::Arc<
    dyn for<'a, 'b, 'c> Fn(&'a str, &'b str, &'c str, f64) -> Option<(String, f64)> + Send + Sync,
>;
/// Per-feature linear blend weights. `None` = SCC default.
#[derive(Clone, Debug, Default)]
// trace:exempt reason=internal-detail
pub struct BlendWeights {
    pub task_ppr: Option<f64>,
    pub global_ppr: Option<f64>,
    pub lexical: Option<f64>,
    pub semantic: Option<f64>,
    pub confidence: Option<f64>,
    pub criticality: Option<f64>,
    pub change_risk: Option<f64>,
    pub novelty: Option<f64>,
}

/// Required-coverage contributor (§124 item 26): symbol ids that MUST
/// count as required (criticality 1.0), unioned with the engine's
/// `required_ids` base. E.g. a security plugin marks tainted sinks
/// required so task ranking never demotes them.
// trace:exempt reason=internal-detail
pub type CoverageProvider =
    Box<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// Rank-time edge contributor (§48): `(subject, predicate, object,
/// weight)` triples that enter diffusion without becoming canonical
/// architecture facts. Every edge records its source (see the
/// `rank-edges(N)` reason); endpoint ids outside the rank universe and
/// non-positive/non-finite weights are skipped by the ranker.
// trace:exempt reason=internal-detail
pub type RankEdgeProvider =
    Box<dyn Fn(&str) -> Vec<(String, String, String, f64)> + Send + Sync>;

/// Rank-universe node contributor (§124 item 17 RankNodeProvider):
/// `(id, kind)` pairs merged into the rank universe before edge indexing.
/// Only rankable kinds enter (the ranker's RANKABLE_KINDS gate); empty ids,
/// unknown kinds, and duplicates of view nodes abstain. Rank-time only:
/// no entity, relationship, or evidence is written. Deterministic chain
/// order; the universe stays id-sorted regardless of provider order.
// trace:exempt reason=internal-detail
pub type RankNodeProvider =
    Box<dyn Fn(&str) -> Vec<(String, String)> + Send + Sync>;

/// Criticality override (§53 CriticalityProvider): per-symbol criticality
/// in [0,1], or `None` to keep the engine default (seed/required =>
/// 1.0, else file-importance score). First `Some` in chain order wins;
/// out-of-range values degrade to the default (never clamp silently into
/// the blend — a contributor that cannot name a valid score abstains).
// trace:exempt reason=internal-detail
pub type CriticalityProvider =
    Box<dyn Fn(&str, &str) -> Option<f64> + Send + Sync>;

/// Novelty override (§53 NoveltyProvider): per-symbol novelty in [0,1],
/// or `None` to keep the engine default of 1.0. Same first-Some-wins and
/// range discipline as [`CriticalityProvider`].
// trace:exempt reason=internal-detail
pub type NoveltyProvider =
    Box<dyn Fn(&str, &str) -> Option<f64> + Send + Sync>;

/// Change-risk override (§53 RiskProvider): per-symbol change risk in
/// [0,1], or `None` to keep the engine default (stale path => 1.0, else
/// 0.0). Same first-Some-wins and range discipline as
/// [`CriticalityProvider`] (shared [`ScalarOverrideProvider`] alias).
// trace:exempt reason=internal-detail
pub type RiskProvider = ScalarOverrideProvider;

/// Semantic-score override (§53 SemanticProvider): per-symbol semantic
/// relevance in [0,1] against the goal, or `None` to keep the engine
/// default of 0.0 (no scorer configured). Same first-Some-wins and range
/// discipline as [`CriticalityProvider`]. When any provider contributes,
/// the blend keeps the documented redistribution math: the semantic
/// share is real, so no renormalization applies.
// trace:exempt reason=internal-detail
pub type SemanticProvider = ScalarOverrideProvider;

/// One scalar-override provider (criticality or novelty): `None` abstains.
// trace:exempt reason=internal-detail
pub type ScalarOverrideProvider =
    Box<dyn Fn(&str, &str) -> Option<f64> + Send + Sync>;

// trace:exempt reason=internal-detail
pub type SimilarityFn = std::sync::Arc<dyn Fn(&str, &str, Option<&str>, Option<&str>) -> f64 + Send + Sync>;

#[derive(Default)]
// trace:exempt reason=internal-detail
pub struct RankHooks {
    pub seed_providers: Vec<SeedProvider>,
    pub features: Vec<RankFeatureFn>,
    pub rerankers: Vec<RerankerFn>,
    pub edge_weights: Vec<EdgeWeightFn>,
    /// Pairwise item similarity for MMR diversification. First provider
    /// returning a nonzero value wins (deterministic chain order);
    /// the built-in default (same-group => 1.0) runs last.
    pub similarities: Vec<SimilarityFn>,
        /// Extra required-coverage providers (§124 item 26): contributed
    /// symbol ids union with the engine `required_ids` base. Recorded in
    /// reasons as `required-by:<n>`.
    pub coverage: Vec<CoverageProvider>,
/// Extra candidate providers (spec 18): merged with the lexical base
    /// by canonical id, max score wins. Deterministic chain order.
    pub candidates: Vec<CandidateProvider>,
    /// Extra rank-time edges (§48): merged into diffusion, never into the
    /// canonical graph. Deterministic chain order.
    pub rank_edges: Vec<RankEdgeProvider>,
    /// Extra rank-universe nodes (§124 item 17): merged before edge
    /// indexing so provider edges can attach to provider nodes.
    /// Deterministic chain order.
    pub rank_nodes: Vec<RankNodeProvider>,
    /// Criticality overrides (§53): first `Some` in chain order wins.
    pub criticality: Vec<CriticalityProvider>,
    /// Novelty overrides (§53): first `Some` in chain order wins.
    pub novelty: Vec<NoveltyProvider>,
    /// Change-risk overrides (§53): first `Some` in chain order wins.
    pub risk: Vec<RiskProvider>,
    /// Semantic-score overrides (§53): first `Some` in chain order wins.
    pub semantic: Vec<SemanticProvider>,
    /// Named blend profiles: profile name -> per-feature weight
    /// overrides for the linear blend (feature keys: task_ppr,
    /// global_ppr, lexical, semantic, confidence, criticality,
    /// change_risk, novelty). Missing keys keep default weights.
    /// Applied inside the same linear math; recorded in reasons.
    pub profiles: std::collections::BTreeMap<String, BlendWeights>,
}

/// First-`Some`-wins override resolution: providers abstain with
/// `None`; out-of-range values ([0,1] required) abstain too. Returns the
/// value plus a `provider(N)` source tag for reasons, or `None` source
/// when every provider abstained and the default stands.
// trace:exempt reason=internal-detail
pub fn resolve_override(
    providers: &[ScalarOverrideProvider],
    symbol: &str,
    goal: &str,
    default: f64,
) -> (f64, Option<String>) {
    for (i, prov) in providers.iter().enumerate() {
        match prov(symbol, goal) {
            Some(v) if v.is_finite() && (0.0..=1.0).contains(&v) => {
                return (v, Some(format!("provider({i})")));
            }
            _ => {}
        }
    }
    (default, None)
}

// trace:exempt reason=internal-detail
pub fn apply_edge_weight(base: f64, mode: &str, value: f64) -> f64 {
    match mode {
        "add" => base + value,
        "multiply" => base * value,
        "replace" => value,
        "veto" => 0.0,
        _ => base,
    }
}


/// One explicit feature row for [`score_entries`]: all eight core
/// inputs plus the task-mode flag, keyed by id.
// trace:exempt reason=internal-detail
pub struct ScoreRow<'a> {
    pub id: &'a str,
    pub task_ppr: f64,
    pub global_ppr: f64,
    pub lexical: f64,
    pub semantic: f64,
    pub confidence: f64,
    pub criticality: f64,
    pub change_risk: f64,
    pub novelty: f64,
    pub has_task: bool,
}

/// Pure per-entry blend (§123 intermediate `ranking.score_entries`):
/// `final_importance` over each explicit row. No store, no hooks — the
/// same math `symbols_with_hooks` blends from, exposed for audit and
/// for callers scoring their own feature rows.
// trace:exempt reason=internal-detail
pub fn score_entries(rows: &[ScoreRow<'_>]) -> Vec<(String, f64)> {
    rows.iter().map(|r| (
        r.id.to_string(),
        scc_context::pagerank::final_importance(
            r.task_ppr, r.global_ppr, r.lexical, r.semantic,
            r.confidence, r.criticality, r.change_risk, r.novelty, r.has_task,
        ),
    )).collect()
}

// trace:exempt reason=internal-detail
pub fn mmr_select(ranked: &[(String, f64)], similar: &dyn Fn(&str, &str) -> f64, lambda: f64, budget: usize) -> Vec<String> {
    scc_context::selector::mmr_diversify(ranked, similar, lambda, budget.max(1))
}

/// Default MMR similarity: same non-empty group => 1.0, else 0.0.
/// Groups are caller-supplied (component/path); `None`/empty never match.
// trace:exempt reason=internal-detail
pub fn default_similarity(a_group: Option<&str>, b_group: Option<&str>) -> f64 {
    match (a_group, b_group) {
        (Some(g1), Some(g2)) if !g1.is_empty() && g1 == g2 => 1.0,
        _ => 0.0,
    }
}

/// Group lookup for a ranked id (parallel groups vec).
// trace:exempt reason=internal-detail
pub fn group_of2<'a>(ranked: &[(String, f64)], groups: &'a [Option<String>], id: &str) -> Option<&'a str> {
    ranked.iter().position(|(rid, _)| rid == id).and_then(|i| groups.get(i).and_then(|g| g.as_deref()))
}

/// Fold chained similarity providers over one pair: first nonzero wins.
/// Falls back to [`default_similarity`] when no provider fires.
// trace:exempt reason=internal-detail
pub fn fold_similarity(
    providers: &[SimilarityFn],
    a: &str,
    b: &str,
    a_group: Option<&str>,
    b_group: Option<&str>,
) -> f64 {
    for p in providers {
        let v = p(a, b, a_group, b_group);
        if v != 0.0 {
            return v.clamp(0.0, 1.0);
        }
    }
    default_similarity(a_group, b_group)
}

// trace:exempt reason=internal-detail
pub fn apply_quotas(items: &[(String, String, f64, usize)], quotas: &[(String, f64)], available_tokens: usize) -> Vec<String> {
    // Same contract as scc-context enforce_quotas (token-fraction caps,
    // rank order preserved, unknown kinds uncapped): reimplemented over
    // owned rows because that signature's `Fn(&str) -> &str` can only
    // derive kinds from the id string itself, not external kind data.
    use std::collections::HashMap;
    let mut caps: HashMap<&str, usize> = HashMap::new();
    for (kind, frac) in quotas {
        caps.insert(kind.as_str(), (frac.clamp(0.0, 1.0) * available_tokens as f64).round() as usize);
    }
    let mut spent: HashMap<&str, usize> = HashMap::new();
    let mut kind_of: HashMap<&str, &str> = HashMap::new();
    let mut cost_of: HashMap<&str, usize> = HashMap::new();
    for (id, kind, _, cost) in items {
        kind_of.insert(id.as_str(), kind.as_str());
        cost_of.insert(id.as_str(), *cost);
    }
    let mut out = Vec::new();
    // Rank order: sort owned indices by value desc, id asc (stable).
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| items[b].2.partial_cmp(&items[a].2).unwrap_or(std::cmp::Ordering::Equal).then_with(|| items[a].0.cmp(&items[b].0)));
    for i in order {
        let (id, _, _, cost) = &items[i];
        let k = kind_of.get(id.as_str()).copied().unwrap_or("core");
        let take = match caps.get(k) {
            None => true,
            Some(&cap) => {
                let s = spent.entry(k).or_insert(0);
                let next = s.saturating_add(*cost);
                if next <= cap { *s = next; true } else { false }
            }
        };
        if take { out.push(id.clone()); }
    }
    out
}

// trace:exempt reason=internal-detail
pub fn select_with_budget(items: &[scc_core::ContextItem], budget: usize, hard_max: usize) -> Vec<usize> {
    scc_context::selector::select_with_budget(items, budget, hard_max)
}
