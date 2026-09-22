//! Engine context operations: every context pack behind one seam.
//!
//! Each function is the body of its `cmd_*` twin with the `println`
//! removed — the engine returns values, transports render them.
//! Byte-identical derivation: same builders, same order, same budgets.

use scc_api::{DetailRequest, ImpactRequest, StartupRequest, StructuralRequest, SurfaceRequest as ApiSurfaceRequest};
use scc_context::startup::{allocate_startup_budget, build_startup, render_startup, task_delta_with_ids, visible_ids_from_startup, StartupContext};
use scc_core::{ContextBudget, SurfaceRenderResult};
use std::path::Path;

// trace:v1 id=impl.crates-scc-engine-src-context.scc-context work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
pub struct SccContext<'a> {
    pub(crate) engine: &'a crate::workspace::Engine<'a>,
}

// trace:exempt reason=internal-detail
pub(crate) fn record_visible_ids(
    led: &mut scc_core::ContextLedger,
    ctx: &scc_context::ContextCompiler,
    ids: &[String],
) {
    for id in ids {
        led.visible_entities.insert(id.clone());
        if let Some(e) = ctx.view.entity(id) {
            match e.kind.as_str() {
                scc_core::kinds::SYMBOL => {
                    led.visible_symbols.insert(id.clone());
                }
                scc_core::kinds::COMPONENT => {
                    led.visible_components.insert(id.clone());
                }
                scc_core::kinds::FLOW => {
                    led.visible_flows.insert(id.clone());
                }
                _ => {}
            }
        }
    }
}

// trace:exempt reason=internal-detail
fn record_rendered_entries(
    led: &mut scc_core::ContextLedger,
    entries: &[scc_core::SurfaceEntry],
    rendered_ids: &[String],
) {
    if !entries.is_empty() {
        for e in entries {
            led.visible_entities.insert(e.symbol_id.clone());
            led.visible_symbols.insert(e.symbol_id.clone());
            led.visible_files.insert(e.path.clone());
            if let Some(c) = &e.component {
                led.visible_components.insert(c.clone());
            }
        }
        return;
    }
    for id in rendered_ids {
        led.visible_entities.insert(id.clone());
    }
}

// trace:v1 id=impl.crates-scc-engine-src-context.scc-context-2 work=WORK-SI-MMMJA4G6 implements=PLAN-SI-SYKFPBEC
impl SccContext<'_> {
    // trace:exempt reason=internal-detail
    pub fn overview(&self) -> crate::Result<scc_context::ContextPack> {
        Ok(self.engine.ctx().system_overview())
    }

    // trace:exempt reason=internal-detail
    pub fn atlas(&self, budget: Option<usize>, full: bool, unbounded: bool) -> crate::Result<scc_context::ContextPack> {
        let ctx = self.engine.ctx();
        Ok(if full {
            ctx.system_atlas_scoped(budget, scc_context::atlas::AtlasScope::Full, unbounded)
        } else if unbounded {
            ctx.system_atlas_scoped(budget, scc_context::atlas::AtlasScope::Production, true)
        } else {
            ctx.system_atlas(budget)
        })
    }

    // trace:exempt reason=internal-detail
    pub fn startup(&self, req: &StartupRequest) -> crate::Result<(StartupContext, String)> {
        let ctx = self.engine.ctx();
        let budget = allocate_startup_budget(&ctx, req.budget);
        let startup = build_startup(&ctx, &budget, scc_context::startup::RENDERER_VERSION);
        let text = render_startup(&startup);
        let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(self.engine.store);
        let mut led = ledger_store.load();
        let (syms, files, comps, flows) = visible_ids_from_startup(&ctx, &startup);
        led.visible_entities.extend(syms.iter().cloned());
        led.visible_symbols.extend(syms);
        led.visible_files.extend(files);
        led.visible_components.extend(comps);
        led.visible_flows.extend(flows);
        ledger_store.save(&led);
        Ok((startup, text))
    }

    // trace:exempt reason=internal-detail
    pub fn surface(&self, req: &ApiSurfaceRequest, semantic: Option<&dyn scc_context::rank::SemanticScorer>) -> crate::Result<(SurfaceRenderResult, String)> {
        let ctx = self.engine.ctx();
        let tokens = req.budget.unwrap_or(ContextBudget::default().surface);
        let request = scc_context::surface::SurfaceRequest {
            mode: match req.task.as_deref() {
                Some(goal) => scc_context::surface::SurfaceMode::Task { goal, visible: None },
                None => scc_context::surface::SurfaceMode::Global,
            },
            budget: tokens,
            explain: req.explain,
            policy: scc_context::surface::SurfacePolicy::defaults(tokens),
            semantic,
        };
        let result = scc_context::surface::build_surface(&ctx, request);
        let text = match req.task.as_deref() {
            Some(goal) => {
                let body = result.text.strip_prefix("SCC SYSTEM SURFACE MAP").unwrap_or(result.text.as_str());
                format!("# SYSTEM SURFACE MAP (task-personalized: {goal}){body}")
            }
            None => result.text.clone(),
        };
        if !result.rendered_ids.is_empty() {
            let store = self.engine.store;
            let mut led = scc_context::context_ledger::ContextLedgerStore::new(store).load();
            record_rendered_entries(&mut led, &result.rendered_entries, &result.rendered_ids);
            scc_context::context_ledger::ContextLedgerStore::new(store).save(&led);
        }
        Ok((result, text))
    }

    // trace:exempt reason=internal-detail
    pub fn component(&self, req: &DetailRequest) -> crate::Result<scc_context::ContextPack> {
        let ctx = self.engine.ctx();
        Ok(if req.unbounded { ctx.component_context_full(&req.id) } else { ctx.component_context(&req.id) })
    }

    // trace:exempt reason=internal-detail
    pub fn flow(&self, req: &DetailRequest) -> crate::Result<scc_context::ContextPack> {
        let ctx = self.engine.ctx();
        Ok(if req.unbounded { ctx.flow_context_full(&req.id) } else { ctx.flow_context(&req.id) })
    }

    // trace:exempt reason=internal-detail
    pub fn impact(&self, req: &ImpactRequest) -> crate::Result<scc_context::ContextPack> {
        let ctx = self.engine.ctx();
        Ok(if req.unbounded {
            ctx.impact_context_full(&req.files, &req.symbols, req.diff.as_deref())
        } else {
            ctx.impact_context(&req.files, &req.symbols, req.diff.as_deref())
        })
    }

    // trace:exempt reason=internal-detail
    pub fn verify(&self, unbounded: bool) -> crate::Result<scc_context::ContextPack> {
        let ctx = self.engine.ctx();
        Ok(if unbounded { ctx.verify_context_full() } else { ctx.verify_context() })
    }

    // trace:exempt reason=internal-detail
    pub fn structural(
        &self,
        req: &StructuralRequest,
        root: &Path,
        semantic: Option<&dyn scc_context::rank::SemanticScorer>,
    ) -> crate::Result<String> {
        let store = self.engine.store;
        let ctx = self.engine.ctx();
        let tokens = req.budget.unwrap_or(ContextBudget::default().structural_source);
        let max_units = (tokens / 1000).clamp(1, 64);
        let paths: Vec<String> = if !req.files.is_empty() {
            let mut resolved = Vec::new();
            for f in &req.files {
                match scc_context::structural_source::resolve_handle_to_path(&store.root, f) {
                    Ok(p) => resolved.push(p),
                    Err(e) => return Ok(format!("# HANDLE REFUSED\n{e}\n")),
                }
            }
            resolved
        } else if let Some(goal) = req.task.as_deref() {
            let goal = goal.trim();
            if goal.is_empty() {
                return Ok("# STRUCTURAL SOURCE\n\nPass --files <paths...> or --task \"<goal>\".".to_string());
            }
            super::task::surface_task_files(&ctx, goal, max_units, tokens.max(1), semantic)
        } else {
            return Ok("# STRUCTURAL SOURCE\n\nPass --files <paths...> or --task \"<goal>\".".to_string());
        };
        if paths.is_empty() {
            return Ok("# STRUCTURAL SOURCE\n\nNo indexed files matched the task goal (run `scc index` first, or pass --files explicitly).".to_string());
        }
        let _ = root;
        let units = scc_context::structural_source::structural_source(&ctx, &paths, max_units);
        if units.is_empty() {
            return Ok("# STRUCTURAL SOURCE\n\nNo indexed symbols in the requested files (run `scc index` first).".to_string());
        }
        Ok(scc_context::structural_source::render_structural(&units))
    }

    // trace:exempt reason=internal-detail
    pub fn task_delta(&self, goal: &str, budget: usize, semantic: Option<&dyn scc_context::rank::SemanticScorer>) -> crate::Result<(String, Vec<String>)> {
        let ctx = self.engine.ctx();
        let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(self.engine.store);
        let visible = ledger_store.load();
        Ok(task_delta_with_ids(&ctx, goal, &visible, budget, semantic))
    }

    // trace:exempt reason=internal-detail
    pub fn record_task_delta_ids(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let ctx = self.engine.ctx();
        let ledger_store = scc_context::context_ledger::ContextLedgerStore::new(self.engine.store);
        let mut led = ledger_store.load();
        record_visible_ids(&mut led, &ctx, ids);
        ledger_store.save(&led);
    }
}
