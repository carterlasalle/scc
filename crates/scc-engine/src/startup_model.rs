//! Structured startup decomposition (spec §75): the startup artifact as
//! addressable model data instead of rendered text only.
//!
//! `StartupContext` already carries atlas text + skeleton + surface +
//! coverage + omissions + artifact; this struct re-exports those fields
//! under stable names so external programs bind to JSON keys instead of
//! scraping `# SECTION` headers. No new derivation — pure projection.

/// Startup artifact as structured data (§75): every section of the
/// rendered startup text addressable by key.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.scc-engine-startup-model work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct StartupModel {
    pub atlas_text: String,
    pub atlas_budget_used: usize,
    pub skeleton: String,
    pub surface_text: String,
    pub important_text: String,
    pub surface_render: scc_core::SurfaceRenderResult,
    pub coverage: Vec<String>,
    pub omissions: Vec<String>,
    pub artifact: scc_core::ContextArtifact,
}

// trace:exempt reason=internal-detail
impl StartupModel {
    // trace:exempt reason=internal-detail
    pub fn from_context(
        ctx: scc_context::startup::StartupContext,
    ) -> Self {
        StartupModel {
            atlas_text: ctx.atlas,
            atlas_budget_used: ctx.atlas_budget_used,
            skeleton: ctx.skeleton,
            surface_text: ctx.surface,
            important_text: ctx.important,
            surface_render: ctx.surface_render,
            coverage: ctx.coverage,
            omissions: ctx.omissions,
            artifact: ctx.artifact,
        }
    }
}
