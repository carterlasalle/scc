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

pub use context::SccContext;
pub use error::{EngineError, Result};
pub use workspace::{Engine, open_engine};
pub use task::{TaskContextArtifact, build_task_context, build_enriched_task_pack};
