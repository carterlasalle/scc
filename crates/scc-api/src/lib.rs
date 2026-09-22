//! SCC versioned request/response contracts.
//!
//! Thin by design: result types ARE the canonical engine types
//! (re-exported, never re-modelled), and request types are the exact
//! argument tuples every transport already passes to the builders.
//! Serialization is the only difference between transports — never
//! semantic content. (`JsonSchema` derives arrive with the schemars
//! dependency; adding a derive never changes the JSON wire shape.)

use serde::{Deserialize, Serialize};

/// Operation API version negotiated per request.
// trace:exempt reason=internal-detail
pub type ApiVersion = u32;

/// The current operation API version.
// trace:exempt reason=internal-detail
pub const API_VERSION: ApiVersion = 1;

/// SCC plugin API compatibility version.
// trace:exempt reason=internal-detail
pub const PLUGIN_API_VERSION: ApiVersion = 1;

// ---------------------------------------------------------------------------
// Canonical result types (single model — re-exported, not redefined).
// ---------------------------------------------------------------------------

// trace:exempt reason=internal-detail
pub use scc_context::ContextPack;
// trace:exempt reason=internal-detail
pub use scc_core::{SurfaceRenderResult, SystemAtlas, SystemIr, SystemSurfaceMap};

// ---------------------------------------------------------------------------
// Envelope: every operation response carries model identity.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ModelIdentity {
    pub epoch: String,
    pub graph_revision: i64,
    pub repository_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct OperationResponse<T> {
    pub operation: String,
    pub api_version: ApiVersion,
    pub scc_version: String,
    pub model: ModelIdentity,
    pub output: T,
}

// ---------------------------------------------------------------------------
// Requests: one struct per operation family, mirroring the CLI args.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct TaskContextRequest {
    pub goal: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub budget: Option<usize>,
    #[serde(default)]
    pub hook: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct StartupRequest {
    #[serde(default)]
    pub budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SurfaceRequest {
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub budget: Option<usize>,
    #[serde(default)]
    pub explain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct DetailRequest {
    pub id: String,
    #[serde(default)]
    pub unbounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ImpactRequest {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub diff: Option<String>,
    #[serde(default)]
    pub unbounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct StructuralRequest {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct QueryRequest {
    pub query: String,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct DiffRequest {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SnapshotSaveRequest {
    pub task: String,
    #[serde(default)]
    pub budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct ExportRequest {
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct IndexPathsRequest {
    pub paths: Vec<String>,
    #[serde(default)]
    pub quiet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankRequest {
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub explain: bool,
    #[serde(default)]
    pub include_features: bool,
    #[serde(default)]
    pub include_intermediate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankItem {
    pub id: String,
    pub rank: f64,
    pub position: usize,
    pub features: RankFeatures,
    pub specificity: f64,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub plugin_features: std::collections::BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankFeatures {
    pub task_ppr: f64,
    pub global_ppr: f64,
    pub lexical: f64,
    pub semantic: f64,
    pub confidence: f64,
    pub criticality: f64,
    pub change_risk: f64,
    pub novelty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankResult {
    pub items: Vec<RankItem>,
    #[serde(default)]
    pub omitted_ids: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct SelectionRequest {
    pub ranked: Vec<RankedEntry>,
    pub budget: usize,
    #[serde(default)]
    pub lambda: Option<f64>,
    #[serde(default)]
    pub quotas: Option<Vec<QuotaEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct RankedEntry {
    pub id: String,
    pub value: f64,
    #[serde(default)]
    pub token_cost: usize,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// trace:exempt reason=internal-detail
pub struct QuotaEntry {
    pub kind: String,
    pub fraction: f64,
}
