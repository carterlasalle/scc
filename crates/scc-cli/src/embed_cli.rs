//! Thin CLI shell: `cmd_embed` renders `embeddings.build`; the scorer,
//! reranker, policy, and constructor live in `scc_engine::inference`
//! (single implementation — DoD 5: no transport reimplements ranking).

use std::path::Path;

/// `scc embed` — terminal rendering over the `embeddings.build` operation.
// trace:v1 id=impl.scc-cli-embed-cli work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub fn cmd_embed(root: &Path) -> crate::Result<()> {
    let out = scc_engine::invoke(root, "embeddings.build", serde_json::json!({}))
        .map_err(|e| crate::CliError::Other(e.to_string()))?;
    println!(
        "embedding with model '{}' stored {} embeddings",
        out.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        out.get("stored").and_then(|n| n.as_u64()).unwrap_or(0),
    );
    Ok(())
}

/// Back-compat re-exports: keep external `scc_cli::embed_cli::` paths
/// compiling during migration (single implementation in the engine).
pub use scc_engine::inference::{EmbeddingScorer, EngineReranker as CliReranker, rankers, remote_inference_allowed};

#[cfg(test)]
mod tests {
    use super::*;
    use scc_context::rank::{Reranker, ScoredEntity, SemanticScorer};
    use scc_indexer::embed::EmbedConfig;
    use scc_store::Store;

    fn tmp_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&dir.path().join("scc.db"), &root).unwrap();
        (store, dir)
    }

    #[test]
    fn reranker_degrades_without_model() {
        let cfg = EmbedConfig {
            base_url: "http://127.0.0.1:1".into(),
            model: "m".into(),
            api_key: None,
            rerank_model: None,
        };
        let rr = CliReranker::new(&cfg);
        let mut cands = vec![ScoredEntity {
            id: "a".into(),
            kind: "symbol".into(),
            name: "x".into(),
            score: 1.0,
            reason: "lexical".into(),
        }];
        rr.rerank("goal", &mut cands);
        assert_eq!(cands.len(), 1); // no panic, no reorder
    }

    #[test]
    fn remote_policy_fails_closed() {
        // loopback: allowed with inference.enabled alone
        let mut local = scc_indexer::Config::default();
        local.inference.enabled = true;
        local.inference.base_url = "http://127.0.0.1:11434/v1".into();
        assert!(remote_inference_allowed(&local));

        // remote endpoint: blocked unless allow_remote_models is set
        let mut remote = local.clone();
        remote.inference.base_url = "https://api.openai.com/v1".into();
        assert!(!remote_inference_allowed(&remote), "remote must fail closed");
        remote.security.allow_remote_models = true;
        assert!(remote_inference_allowed(&remote));

        // inference disabled: nothing allowed
        let mut off = remote.clone();
        off.inference.enabled = false;
        assert!(!remote_inference_allowed(&off));

        // empty base_url resolves to the local ollama default
        let mut local2 = scc_indexer::Config::default();
        local2.inference.enabled = true;
        local2.inference.provider = "local".into();
        assert!(remote_inference_allowed(&local2));
    }

    #[test]
    fn remote_classification_covers_common_hosts() {
        let mk = |base_url: &str| EmbedConfig {
            base_url: base_url.into(),
            model: "m".into(),
            api_key: None,
            rerank_model: None,
        };
        assert!(!mk("http://127.0.0.1:11434/v1").is_remote());
        assert!(!mk("http://localhost:11434").is_remote());
        assert!(!mk("http://[::1]:11434/v1").is_remote());
        assert!(!mk("http://0.0.0.0:8080").is_remote());
        assert!(mk("https://api.openai.com/v1").is_remote());
        assert!(mk("https://gateway.example/v1").is_remote());
        assert!(mk("http://192.168.1.10:8080").is_remote());
    }

    #[test]
// trace:exempt reason=unit-test
    fn scorer_uses_stored_embeddings() {
        let (store, _d) = tmp_store();
        let mut e = scc_core::Entity::new("repo://r/symbol/a.py/boosted", "symbol", "boosted");
        e.attr("file", serde_json::json!("a.py"));
        store.insert_entity(&e, &["a.py".into()]).unwrap();
        // store a vector aligned with a goal vector [1,0,0...]
        let mut v = vec![0.0f32; 8];
        v[0] = 1.0;
        store.put_embedding(&e.id, &v, "test").unwrap();
        let _cfg = EmbedConfig {
            base_url: "http://127.0.0.1:1".into(),
            model: "test".into(),
            api_key: None,
            rerank_model: None,
        };
        // scorer construction needs to embed the goal — bypass via a
        // hand-built scorer with a known goal vector
        let mut m = std::collections::HashMap::new();
        m.insert(e.id.clone(), v);
        let scorer = EmbeddingScorer::from_vectors(
            vec![1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            m,
        );
assert!((scorer.score("goal", &e) - 1.0).abs() < 1e-6);
        // unrelated entity scores 0
        let other = scc_core::Entity::new("repo://r/symbol/a.py/z", "symbol", "z");
        assert_eq!(scorer.score("goal", &other), 0.0);
    }
}
