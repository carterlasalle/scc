//! SCC engine facade: one orchestration seam for every transport.
//!
//! The `scc` CLI is one client of this engine — never the only way to
//! access functionality. CLI, HTTP, MCP, SDKs, RPC, and FFI all call
//! these operations; business logic lives here, transports only parse
//! arguments and render results.
//!
//! Migration rule (spec §50): installing no third-party plugins preserves
//! existing behavior. This facade moves orchestration WITHOUT changing
//! algorithms — outputs stay byte-identical (parity tests enforce it).

// trace:exempt reason=module-facade
pub mod context;
// trace:exempt reason=module-facade
pub mod error;
// trace:exempt reason=module-facade
pub mod workspace;
// trace:exempt reason=module-facade
pub mod task;
// trace:exempt reason=module-facade
pub mod checkpoint;
// trace:exempt reason=module-facade
pub mod graph;
// trace:exempt reason=module-facade
pub mod index;
// trace:exempt reason=module-facade
pub mod ops;
// trace:exempt reason=module-facade
pub mod plugins;
// trace:exempt reason=module-facade
pub mod rpc;
// trace:exempt reason=module-facade
pub mod snapshots;
// trace:exempt reason=module-facade
pub mod state;
// trace:exempt reason=module-facade
pub mod status;
// trace:exempt reason=module-facade
pub mod systems;
// trace:exempt reason=module-facade
pub mod exports;
pub mod diagram;
// trace:exempt reason=module-facade
pub mod integrations;
// trace:exempt reason=module-facade
pub mod history;
// trace:exempt reason=module-facade
pub mod inference;
// trace:exempt reason=module-facade
pub mod invoke;
// trace:exempt reason=module-facade
pub mod misc;
// trace:exempt reason=module-facade
pub mod ranking;



pub use context::SccContext;
pub use error::{EngineError, Result};
pub use workspace::{Engine, open_engine};
pub use invoke::invoke;
pub use ops::{describe as describe_operation, OPERATIONS};
pub use ranking::{RankFeatureValue, RankHooks};
pub use task::{TaskContextArtifact, build_task_context, build_enriched_task_pack};
