//! Engine task artifact: THE one complete task derivation.
//!
//! Moved verbatim from `scc-cli` commands.rs: `build_task_context` is the
//! transport-parity root — CLI text, CLI --json, MCP, HTTP, Hermes, SDKs,
//! and hooks all derive from this ONE builder; outputs differ in
//! serialization only, never in semantic content. The scorer is resolved
//! ONCE and feeds BOTH the pack rankers and the delta Surface request.

use scc_api::TaskContextRequest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:exempt reason=internal-detail
pub struct TaskContextArtifact {
    pub pack: scc_context::ContextPack,
    pub delta: String,
    #[serde(default)]
    pub delta_ids: Vec<String>,
    #[serde(default)]
    pub token_count: usize,
}

// trace:exempt reason=internal-detail
pub(crate) fn surface_task_files(
    ctx: &scc_context::ContextCompiler,
    goal: &str,
    limit: usize,
    tokens: usize,
    semantic: Option<&dyn scc_context::rank::SemanticScorer>,
) -> Vec<String> {
    let budget = tokens.max(1);
    let request = scc_context::surface::SurfaceRequest {
        mode: scc_context::surface::SurfaceMode::Task { goal, visible: None },
        budget,
        explain: false,
        policy: scc_context::surface::SurfacePolicy::defaults(budget),
        semantic,
    };
    let result = scc_context::surface::build_surface(ctx, request);
    let by_id: BTreeMap<&str, &scc_core::SurfaceEntry> = result
        .rendered_entries
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let mut files: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for id in &result.rendered_ids {
        let path = by_id.get(id.as_str()).map(|e| e.path.as_str()).unwrap_or("");
        if path.is_empty() || !seen.insert(path.to_string()) {
            continue;
        }
        files.push(path.to_string());
        if files.len() >= limit {
            break;
        }
    }
    files
}

// trace:v1 id=impl.crates-scc-engine-src-task.truncate-to work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub(crate) fn truncate_to(content: &str, cap: usize) -> String {
    if scc_core::estimate_tokens(content) <= cap {
        return content.to_string();
    }
    const FOOTER: &str = "\n\n\u{2026} [task hard cap: content truncated to fit budget]\n";
    let footer_tokens = scc_core::estimate_tokens(FOOTER);
    if footer_tokens > cap {
        return String::new();
    }
    let target = cap.saturating_sub(footer_tokens);
    let mut lo = 0usize;
    let mut hi = content.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let prefix = &content[..content.floor_char_boundary(mid)];
        if scc_core::estimate_tokens(prefix) <= target {
            lo = mid;
        } else {
            hi = mid.saturating_sub(1);
        }
    }
    let mut cut = content.floor_char_boundary(lo);
    let min_chars = content[..cut].chars().count() / 2;
    if let Some(nl) = content[..cut].rfind('\n') {
        let prefix_chars = content[..nl].chars().count();
        if prefix_chars >= min_chars {
            cut = nl;
        }
    }
    if let Some(rest) = content.get(cut..) {
        if let Some(nl) = rest.find('\n') {
            let extended = cut + nl;
            let mut candidate = content[..extended].to_string();
            candidate.push_str(FOOTER);
            if scc_core::estimate_tokens(&candidate) <= cap {
                cut = extended;
            }
        }
    }
    let mut out = content[..cut].to_string();
    out.push_str(FOOTER);
    while scc_core::estimate_tokens(&out) > cap && cut > 0 {
        cut = content.floor_char_boundary(cut.saturating_sub(1));
        if let Some(nl) = content[..cut].rfind('\n') {
            cut = nl;
        }
        out = content[..cut].to_string();
        out.push_str(FOOTER);
        if cut == 0 {
            break;
        }
    }
    out
}

// trace:exempt reason=internal-detail
pub struct EnrichedPack {
    pub pack: scc_context::ContextPack,
    pub base: String,
    pub beads: String,
    pub hindsight: String,
}

// trace:v1 id=impl.crates-scc-engine-src-task.enriched-pack work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
impl EnrichedPack {
    // trace:exempt reason=internal-detail
    pub fn tokens_including(&self, hindsight: bool, beads: bool) -> usize {
        scc_core::estimate_tokens(&self.base)
            + if beads { scc_core::estimate_tokens(&self.beads) } else { 0 }
            + if hindsight { scc_core::estimate_tokens(&self.hindsight) } else { 0 }
    }

    // trace:exempt reason=internal-detail
    pub fn assemble(&mut self, hindsight: bool, beads: bool) {
        self.pack.content = String::new();
        self.pack.content.push_str(&self.base);
        if beads {
            self.pack.content.push_str(&self.beads);
        }
        if hindsight {
            self.pack.content.push_str(&self.hindsight);
        }
        self.pack.tokens = scc_core::estimate_tokens(&self.pack.content);
    }
}

#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
pub fn enrich_task_pack(
    engine: &crate::workspace::Engine,
    config: &scc_indexer::Config,
    root: &Path,
    goal: &str,
    files: &[String],
    symbols: &[String],
    budget: Option<usize>,
    hook: bool,
    scorer: Option<&dyn scc_context::rank::SemanticScorer>,
    reranker: Option<&dyn scc_context::rank::Reranker>,
) -> EnrichedPack {
    let mut pack = engine.ctx().task_context_with_rankers(
        goal,
        files,
        symbols,
        if hook { Some(budget.unwrap_or(1500).min(1500)) } else { budget },
        scorer,
        reranker,
    );
    let mut beads = String::new();
    let beads_active = scc_indexer::adapters::beads::active_beads(root, 5);
    if !beads_active.is_empty() {
        beads.push_str("\n# ACTIVE TASK STATE (from .beads/issues.jsonl \u{2014} task state, not system facts)\n");
        for t in beads_active {
            beads.push_str("- ");
            beads.push_str(&t);
            beads.push('\n');
        }
    }
    let mut hindsight = String::new();
    if config.integrations.hindsight {
        let lessons = scc_indexer::adapters::hindsight::lessons(engine.store, 5);
        if !lessons.is_empty() {
            hindsight.push_str("\n# HINDSIGHT LESSONS (memory, below System IR authority \u{2014} not verified facts)\n");
            for (content, tags) in lessons {
                let tag_str = if tags.is_empty() { String::new() } else { format!(" [{}]", tags.join(", ")) };
                hindsight.push_str(&format!("- {content}{tag_str}\n"));
            }
        }
    }
    let sections = crate::plugins::context_sections(root, config, goal, files, symbols);
    let base = pack.content.clone();
    pack.content.push_str(&sections);
    pack.content.push_str(&beads);
    pack.content.push_str(&hindsight);
    pack.tokens = scc_core::estimate_tokens(&pack.content);
    // Sections are pack content (same authority line as beads/hindsight):
    // fold into base so assemble() keeps them under budget accounting.
    let mut base_with_sections = base;
    base_with_sections.push_str(&sections);
    EnrichedPack { pack, base: base_with_sections, beads, hindsight }
}

/// THE one complete task artifact builder. Transports pass the opened
/// engine + resolved scorer/reranker; the engine derives pack + delta +
/// ledger + cap enforcement identically for every caller.
#[allow(clippy::too_many_arguments)]
// trace:exempt reason=internal-detail
pub fn build_task_context(
    engine: &crate::workspace::Engine,
    config: &scc_indexer::Config,
    root: &Path,
    req: &TaskContextRequest,
    scorer: Option<&dyn scc_context::rank::SemanticScorer>,
    reranker: Option<&dyn scc_context::rank::Reranker>,
) -> crate::Result<TaskContextArtifact> {
    let mut ep = enrich_task_pack(engine, config, root, &req.goal, &req.files, &req.symbols, req.budget, req.hook, scorer, reranker);
    let hard_cap: Option<usize> = if req.hook { Some(req.budget.unwrap_or(1500).min(1500)) } else { req.budget };
    let ctx = engine.ctx();
    let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(engine.store);
    let visible = ledger_store.load();
    let delta_budget = match hard_cap {
        Some(c) => c.saturating_sub(ep.pack.tokens),
        None => scc_core::ContextBudget::default().task_delta,
    };
    let (mut delta, mut delta_ids) = scc_context::startup::task_delta_with_ids(&ctx, &req.goal, &visible, delta_budget, scorer);
    let mut include_hindsight = true;
    let mut include_beads = true;
    let mut dropped: Vec<&str> = Vec::new();
    if let Some(cap) = hard_cap {
        let full = ep.tokens_including(true, true) + scc_core::estimate_tokens(&delta);
        if full > cap && include_hindsight && !ep.hindsight.is_empty() {
            include_hindsight = false;
            dropped.push("hindsight");
        }
        let no_h = ep.tokens_including(include_hindsight, true) + scc_core::estimate_tokens(&delta);
        if no_h > cap && include_beads && !ep.beads.is_empty() {
            include_beads = false;
            dropped.push("beads");
        }
        let no_e = ep.tokens_including(include_hindsight, include_beads) + scc_core::estimate_tokens(&delta);
        if no_e > cap && !delta.is_empty() {
            delta = String::new();
            delta_ids.clear();
            dropped.push("surface-delta");
        }
    }
    ep.assemble(include_hindsight, include_beads);
    if let Some(cap) = hard_cap {
        let total = scc_core::estimate_tokens(&ep.pack.content) + scc_core::estimate_tokens(&delta);
        if total > cap {
            ep.pack.content = truncate_to(&ep.pack.content, cap);
            ep.pack.tokens = scc_core::estimate_tokens(&ep.pack.content);
            ep.pack.hard_truncated = true;
            dropped.push("task-pack");
        }
    }
    if !delta_ids.is_empty() {
        let mut led = visible;
        crate::context::record_visible_ids(&mut led, &ctx, &delta_ids);
        ledger_store.save(&led);
    }
    let token_count = scc_core::estimate_tokens(&ep.pack.content) + scc_core::estimate_tokens(&delta);
    let mut artifact = TaskContextArtifact { pack: ep.pack, delta, delta_ids, token_count };
    if let Some(cap) = hard_cap {
        assert!(artifact.token_count <= cap, "task artifact {token_count} exceeded hard cap {cap}");
    }
    if !dropped.is_empty() {
        artifact.pack.warnings.push(format!("task cap enforced: dropped [{}]", dropped.join(", ")));
    }
    Ok(artifact)
}

/// Pack-only builder: NO delta, NO ledger mutation. Compress and other
/// pack-only callers MUST use this — never `build_task_context`.
// trace:exempt reason=internal-detail
pub fn build_enriched_task_pack(
    engine: &crate::workspace::Engine,
    config: &scc_indexer::Config,
    root: &Path,
    req: &TaskContextRequest,
    scorer: Option<&dyn scc_context::rank::SemanticScorer>,
    reranker: Option<&dyn scc_context::rank::Reranker>,
) -> crate::Result<scc_context::ContextPack> {
    Ok(enrich_task_pack(engine, config, root, &req.goal, &req.files, &req.symbols, req.budget, req.hook, scorer, reranker).pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc-engine-task.truncate-to-hard-cap work=WORK-task-context-transport-parity verifies=REQ-complete-task-context-identical-across-transports,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.crates-scc-engine-src-task.truncate-to
    fn truncate_to_never_exceeds_hard_cap_when_next_newline_is_far_past() {
        // Binary search finds a fitting prefix, then the old implementation
        // extended FORWARD to the next newline (`cut += nl`), which blew
        // past the cap when that newline was far away. A true hard cap
        // must back off (or re-check after extending).
        let mut body = String::from("HEADER\n");
        body.push_str(&"x".repeat(400)); // ~100 tokens before the next newline
        body.push('\n');
        body.push_str("TAIL\n");
        let cap = 40; // far below the long line
        let out = truncate_to(&body, cap);
        let tokens = scc_core::estimate_tokens(&out);
        assert!(
            tokens <= cap,
            "truncate_to must honor the hard cap: {tokens} > {cap}\n{out:?}"
        );
        assert!(
            out.contains("task hard cap"),
            "footer must be present: {out}"
        );
        assert!(
            !out.contains(&"x".repeat(400)),
            "must not include the line that sits far past the cap"
        );
    }

    #[test]
    // trace:v1 id=test.scc-engine-task.truncate-to-fits-already work=WORK-task-context-transport-parity verifies=REQ-complete-task-context-identical-across-transports
    fn truncate_to_returns_original_when_under_cap() {
        let content = "short\n";
        assert_eq!(truncate_to(content, 1000), content);
    }

    #[test]
    // trace:v1 id=test.scc-engine-task.truncate-to-footer-exceeds-cap work=WORK-task-context-transport-parity verifies=REQ-complete-task-context-identical-across-transports,REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.crates-scc-engine-src-task.truncate-to
    fn truncate_to_returns_empty_when_footer_exceeds_cap() {
        let content = "HEADER\nbody that does not fit\n";
        let out = truncate_to(content, 1);
        assert!(
            out.is_empty(),
            "footer larger than the cap must not be returned: {out:?} tokens={}",
            scc_core::estimate_tokens(&out)
        );
    }
}
