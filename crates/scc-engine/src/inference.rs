//! Engine inference rankers (SCC-071): `EmbeddingScorer` fusing stored
//! entity embeddings via cosine similarity, `EngineReranker` calling a
//! separate `/rerank` model, the remote-model policy, and the `rankers`
//! constructor task-pack plumbing uses when `inference.enabled` is set.
//!
//! Single implementation: transports resolve scorer + reranker through
//! ONE engine constructor — no transport reimplements embedding auth
//! policy or fallback semantics.

use scc_context::rank::{Reranker, ScoredEntity, SemanticScorer};
use scc_indexer::embed::{cosine, rerank, EmbedConfig, EMBED_KINDS};
use scc_store::Store;
use std::collections::HashMap;

// trace:v1 id=impl.scc-engine-inference work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
/// Fuses stored entity embeddings with the embedded goal. Vectors are
/// preloaded once per pack generation.
// trace:exempt reason=internal-detail
pub struct EmbeddingScorer {
    goal_vector: Vec<f32>,
    vectors: HashMap<String, Vec<f32>>,
}

// trace:exempt reason=internal-detail
impl EmbeddingScorer {
// trace:exempt reason=internal-detail
    pub fn from_vectors(goal_vector: Vec<f32>, vectors: std::collections::HashMap<String, Vec<f32>>) -> Self {
        EmbeddingScorer { goal_vector, vectors }
    }
// trace:exempt reason=internal-detail
    pub fn new(goal: &str, cfg: &EmbedConfig, store: &Store) -> Result<EmbeddingScorer, String> {
        let vectors = scc_indexer::embed::embed_texts(cfg, &[goal])?;
        let goal_vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| "embedding request returned no vector".to_string())?;
        let mut map = HashMap::new();
        for kind in EMBED_KINDS {
            for e in store.entities_by_kind(kind).map_err(|e| e.to_string())? {
                if let Ok(Some((v, _))) = store.get_embedding(&e.id) {
                    map.insert(e.id, v);
                }
            }
        }
        Ok(EmbeddingScorer {
            goal_vector,
            vectors: map,
        })
    }
}

// trace:exempt reason=internal-detail
impl SemanticScorer for EmbeddingScorer {
// trace:exempt reason=internal-detail
    fn score(&self, _goal: &str, entity: &scc_core::Entity) -> f64 {
        match self.vectors.get(&entity.id) {
            Some(v) => cosine(&self.goal_vector, v),
            None => 0.0,
        }
    }
}

/// Second-stage reranker calling the configured `/rerank` model on the top
/// candidates. Any failure is a no-op (graceful degradation).
// trace:exempt reason=internal-detail
pub struct EngineReranker {
    cfg: EmbedConfig,
}

// trace:exempt reason=internal-detail
impl EngineReranker {
// trace:exempt reason=internal-detail
    pub fn new(cfg: &EmbedConfig) -> EngineReranker {
        EngineReranker { cfg: cfg.clone() }
    }
}

// trace:exempt reason=internal-detail
impl Reranker for EngineReranker {
// trace:exempt reason=internal-detail
    fn rerank(&self, goal: &str, candidates: &mut Vec<ScoredEntity>) {
        if candidates.is_empty() || self.cfg.rerank_model.is_none() {
            return;
        }
        let docs: Vec<String> = candidates
            .iter()
            .take(30)
            .map(|c| format!("{} {}", c.kind, c.name))
            .collect();
        if let Ok(scores) = rerank(&self.cfg, goal, &docs) {
            for (c, s) in candidates.iter_mut().take(30).zip(scores.iter()) {
                // rerank dominates; the lexical residue keeps ties
                // deterministic
                c.score = c.score * 0.2 + s * 5.0;
                c.reason = format!("{} + rerank", c.reason);
            }
            candidates.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        // Err: graceful degrade — keep lexical order
    }
}

/// Remote-model policy (P0, docs/SECURITY.md): repository-derived content
/// may leave the machine only when `inference.enabled` AND
/// `security.allow_remote_models` are both true. Loopback providers need
/// only `inference.enabled`. Fails closed.
// trace:exempt reason=internal-detail
pub fn remote_inference_allowed(config: &scc_indexer::Config) -> bool {
    if !config.inference.enabled {
        return false;
    }
    let cfg = EmbedConfig::from_config(&config.inference);
    !cfg.is_remote() || config.security.allow_remote_models
}

/// Build the scorer/reranker when inference is enabled; any provider failure
/// degrades to (None, None) so the lexical ranker is always the fallback.
// trace:exempt reason=internal-detail
pub fn rankers(
    store: &Store,
    config: &scc_indexer::Config,
    goal: &str,
) -> (Option<EmbeddingScorer>, Option<EngineReranker>) {
    if !remote_inference_allowed(config) {
        if config.inference.enabled {
            eprintln!(
                "scc: warning: remote inference blocked by security policy \
                 (security.allow_remote_models is false); using lexical ranking only"
            );
        }
        return (None, None);
    }
    let cfg = EmbedConfig::from_config(&config.inference);
    let scorer = EmbeddingScorer::new(goal, &cfg, store).ok();
    let reranker = if cfg.rerank_model.is_some() {
        Some(EngineReranker::new(&cfg))
    } else {
        None
    };
    (scorer, reranker)
}
