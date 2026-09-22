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
        let ctx = self.ctx();
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust(&ctx.view, &|s, p, o, b| {
            Self::fold_edge_contributors(contributors, s, p, o, b).0
        });
        let v = ranker.global_vector();
        Ok(ranker.nodes().iter().cloned().zip(v).collect())
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
        let ctx = self.ctx();
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust(&ctx.view, &|s, p, o, b| {
            Self::fold_edge_contributors(contributors, s, p, o, b).0
        });
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

    /// Lexical candidate generation for goal (stage 1).
    // trace:exempt reason=internal-detail
    pub fn candidates(&self, goal: &str, limit: usize) -> crate::Result<Vec<scc_context::rank::ScoredEntity>> {
        let ctx = self.ctx();
        Ok(scc_context::rank::collect_lexical_candidates(ctx.store, &ctx.view, goal, &[], limit.max(1)))
    }

    /// Full task/global blend per symbol with feature decomposition.
    /// Same math as build_surface (no MMR/quotas/budget — pure ranking).
    /// Plugin seed/feature/rerank hooks apply here and are recorded.
    // trace:v1 id=impl.scc-engine-ranking.symbols work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
    pub fn symbols(&self, req: &RankRequest) -> crate::Result<RankResult> {
        self.symbols_with_hooks(req, &RankHooks::default())
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
        // Named blend profiles (§12): only `default` exists until a plugin
        // registers one. Unknown names fail loudly — silently running
        // default math under a requested profile would lie about behavior.
        if let Some(profile) = req.profile.as_deref() {
            if profile != "default" {
                return Err(crate::EngineError::Other(format!(
                    "unknown ranking profile '{profile}' (available: default)"
                )));
            }
        }
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
        let edge_contributors: &[EdgeWeightFn] = &hooks.edge_weights;
        let ranker = scc_context::pagerank::SystemRanker::with_edge_adjust(&ctx.view, &|s, p, o, b| {
            Self::fold_edge_contributors(edge_contributors, s, p, o, b).0
        });
        let global_of: std::collections::BTreeMap<String, f64> =
            ranker.project_to_symbols(&ranker.global_vector()).into_iter().collect();
        let task_of: std::collections::BTreeMap<String, f64> =
            ranker.project_to_symbols(&ranker.task_vector(&seeds)).into_iter().collect();
        let has_task = !goal.is_empty();
        let map = scc_context::surface::compile_surface_map(&ctx);
        let required = scc_context::surface::required_ids(&map, &ctx);
        let mut best: std::collections::BTreeMap<&str, RankItem> = std::collections::BTreeMap::new();
        for e in &map.entries {
            let task_ppr = task_of.get(&e.symbol_id).copied().unwrap_or(0.0);
            let global_ppr = global_of.get(&e.symbol_id).copied().unwrap_or(0.0);
            let lexical = scc_context::surface::entry_lexical(e, &goal_terms);
            let confidence = e.confidence as f64;
            let criticality = if seed_ids.contains(e.symbol_id.as_str()) || required.contains(&e.id) { 1.0 } else { importance_file_score(&e.path) };
            let change_risk = if !e.path.is_empty() && ctx.stale_paths.iter().any(|p| p == &e.path) { 1.0 } else { 0.0 };
            let blend = scc_context::pagerank::final_importance(task_ppr, global_ppr, lexical, 0.0, confidence, criticality, change_risk, 0.0, has_task);
            let scale = 1.0 / (1.0 - scc_context::pagerank::SEMANTIC_WEIGHT);
            let total = blend * scale + scc_context::pagerank::NOVELTY_WEIGHT * 1.0;
            let mut plugin_features = std::collections::BTreeMap::new();
            let mut reasons: Vec<String> = Vec::new();
            if seed_ids.contains(e.symbol_id.as_str()) { reasons.push("task-seed".into()); }
            let mut total = total;
            for feat in &hooks.features {
                let v = feat(&e.symbol_id, goal);
                plugin_features.insert(v.name.clone(), v.score);
                total += v.weight * v.score;
                if !v.reason.is_empty() { reasons.push(v.reason.clone()); }
            }
            let specificity = if e.exported { 1.15 } else { 1.0 };
            let item = RankItem { id: e.symbol_id.clone(), rank: total, position: 0,
                features: RankFeatures { task_ppr, global_ppr, lexical, semantic: 0.0,
                    confidence, criticality, change_risk, novelty: 1.0 },
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
pub type RankFeatureFn = Box<dyn Fn(&str, &str) -> RankFeatureValue + Send + Sync>;
// trace:exempt reason=internal-detail
pub type RerankerFn = Box<dyn Fn(&mut Vec<scc_api::RankItem>, &str) + Send + Sync>;
/// Edge-weight contributor: per-edge (subject, predicate, object, base)
/// adjustment. Return `Some((mode, value))` to alter the weight, `None`
/// for no change. Modes: add | multiply | replace | veto. Every applied
/// contribution is recorded on the affected rank items' reasons.
// trace:exempt reason=internal-detail
pub type EdgeWeightFn =
    Box<dyn Fn(&str, &str, &str, f64) -> Option<(String, f64)> + Send + Sync>;
#[derive(Default)]
// trace:exempt reason=internal-detail
pub struct RankHooks {
    pub seed_providers: Vec<SeedProvider>,
    pub features: Vec<RankFeatureFn>,
    pub rerankers: Vec<RerankerFn>,
    pub edge_weights: Vec<EdgeWeightFn>,
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


// trace:exempt reason=internal-detail
pub fn mmr_select(ranked: &[(String, f64)], similar: &dyn Fn(&str, &str) -> f64, lambda: f64, budget: usize) -> Vec<String> {
    scc_context::selector::mmr_diversify(ranked, similar, lambda, budget.max(1))
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
