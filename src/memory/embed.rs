use std::sync::{Arc, Mutex};

use crate::error::{CatClawError, Result};

/// Wrapper around fastembed's TextEmbedding for BGE-M3 bilingual embedding.
/// Supports 100+ languages including Chinese and English (1024 dims, 8192 context).
/// Model is downloaded on first use (~560MB) and cached in ~/.cache/huggingface/.
pub struct Embedder {
    model: Arc<Mutex<fastembed::TextEmbedding>>,
    /// Caps concurrent embedding inference to 1. BGE-M3 inference is CPU- and
    /// RAM-heavy; on small VMs N concurrent `memory_write`s would stack N
    /// spikes and push the box into swap thrash (seen in prod 2026-05-13).
    /// Serializing is also fine for throughput — the model is behind a Mutex
    /// anyway, so callers already wait; the semaphore just makes them wait in
    /// the async layer instead of pinning a blocking thread on a contended
    /// lock.
    inference_gate: Arc<tokio::sync::Semaphore>,
}

impl Embedder {
    /// Create a new embedder with BGE-M3 model.
    /// This triggers model download on first run.
    #[allow(dead_code)]
    pub fn new() -> Result<Self> {
        // Use absolute cache dir under ~/.catclaw/models/ to avoid relative path issues
        // when running as background daemon (cwd may differ).
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let cache_dir = std::path::PathBuf::from(home).join(".catclaw").join("models");
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            return Err(CatClawError::Memory(format!("failed to create model cache dir: {}", e)));
        }

        // Only show download progress when stdout is a TTY (foreground mode).
        let show_progress = atty::is(atty::Stream::Stdout);
        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(fastembed::EmbeddingModel::BGEM3)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(show_progress),
        )
        .map_err(|e| CatClawError::Memory(format!("failed to init embedding model: {}", e)))?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            inference_gate: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    /// Embed one or more texts. Returns one Vec<f32> per input text.
    /// Acquires the inference gate (max 1 concurrent) before doing CPU-bound
    /// work on a blocking thread, so concurrent callers queue in the async
    /// layer rather than stacking blocking threads on a contended Mutex.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let model = self.model.clone();
        let _permit = self
            .inference_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| CatClawError::Memory(format!("embedding gate closed: {}", e)))?;
        tokio::task::spawn_blocking(move || {
            let mut m = model.lock().unwrap();
            m.embed(&owned, None)
                .map_err(|e| CatClawError::Memory(format!("embedding failed: {}", e)))
        })
        .await
        .map_err(|e| CatClawError::Memory(format!("embed task join failed: {}", e)))?
        // _permit drops here, releasing the gate
    }

    /// Embed a single text.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| CatClawError::Memory("embedding returned empty result".to_string()))
    }
}
