//! Engine error: the single error type every transport renders.

#[derive(Debug, thiserror::Error)]
// trace:exempt reason=internal-detail
pub enum EngineError {
    #[error("store: {0}")]
    Store(#[from] scc_store::StoreError),
    #[error("index: {0}")]
    Index(#[from] scc_indexer::IndexError),
    #[error("graph: {0}")]
    Graph(#[from] scc_graph::GraphError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(#[from] scc_indexer::config::ConfigError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// trace:exempt reason=internal-detail
pub type Result<T> = std::result::Result<T, EngineError>;
