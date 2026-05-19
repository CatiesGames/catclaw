use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use tracing::{info, warn};

use crate::error::{CatClawError, Result};

/// Wrapper around fastembed's TextEmbedding for BGE-M3 bilingual embedding.
/// Supports 100+ languages including Chinese and English (1024 dims, 8192 context).
/// Model is downloaded on first use (~2.3 GB) and cached in ~/.catclaw/models/.
///
/// **Loading strategy: owned bytes, not mmap.**
/// fastembed's default `try_new` constructor uses `commit_from_file`, which
/// makes ONNX Runtime mmap the model file. On a small VM (8 GiB) where
/// catclaw + claude subprocesses + docker compete for RAM, kernel routinely
/// evicts those mmap pages whenever anon memory grows even a little. Each
/// subsequent inference then page-faults the entire ~2.27 GiB back from
/// disk — a single `memory_write` storm could rack up 100+ GiB of disk
/// reads (incident 2026-05-19).
///
/// To defeat that, we read the three model files (main graph + external
/// weights + the auxiliary `Constant_7_attr__value` blob) into owned
/// `Vec<u8>` buffers and hand them to `try_new_from_user_defined`. ort
/// routes them through `CreateSessionFromArray`, which deserializes the
/// weights into the runtime's own heap allocations — anonymous memory the
/// kernel can't drop for free. RSS jumps by ~2.27 GiB (visible in monitors)
/// but page-cache thrash is gone for good.
pub struct Embedder {
    model: Arc<Mutex<TextEmbedding>>,
    /// Caps concurrent embedding inference to 1. BGE-M3 inference is CPU- and
    /// RAM-heavy; on small VMs N concurrent `memory_write`s would stack N
    /// spikes and push the box into swap thrash (seen in prod 2026-05-13).
    /// Serializing is also fine for throughput — the model is behind a Mutex
    /// anyway, so callers already wait; the semaphore just makes them wait in
    /// the async layer instead of pinning a blocking thread on a contended
    /// lock.
    inference_gate: Arc<tokio::sync::Semaphore>,
}

/// Files referenced by `BAAI/bge-m3` ONNX in fastembed's model registry.
/// Order matters: main graph first (`model.onnx`), then the two referenced
/// external initializers. Tokenizer files are loaded separately.
///
/// Mirror of `src/models/text_embedding.rs::BGEM3` in fastembed 5.x —
/// kept here as constants so we don't depend on internal fastembed
/// APIs to enumerate them.
const BGEM3_ONNX_RELATIVE: &str = "onnx/model.onnx";
const BGEM3_ONNX_EXTERNAL_FILES: &[&str] = &["model.onnx_data", "Constant_7_attr__value"];

impl Embedder {
    /// Create a new embedder with BGE-M3 model.
    ///
    /// Two-phase load:
    ///   1. Call `try_new` once so fastembed downloads + verifies the cache
    ///      (we let it own the hf-hub / hash / retry plumbing). The session
    ///      from this phase is dropped immediately — its mmap mapping goes
    ///      with it.
    ///   2. Locate the cache snapshot dir, slurp the 3 ONNX files + 4
    ///      tokenizer files into owned `Vec<u8>`, and rebuild the
    ///      `TextEmbedding` via `try_new_from_user_defined`. Now the model
    ///      lives in anon heap, not in file-backed page cache.
    ///
    /// If phase 2 fails (cache layout changed, file missing, etc.) we log
    /// a warning and fall back to the phase-1 mmap session. Better to ship
    /// degraded than to brick memory palace.
    #[allow(dead_code)]
    pub fn new() -> Result<Self> {
        // Use absolute cache dir under ~/.catclaw/models/ to avoid relative path issues
        // when running as background daemon (cwd may differ).
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let cache_dir = PathBuf::from(home).join(".catclaw").join("models");
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            return Err(CatClawError::Memory(format!(
                "failed to create model cache dir: {}",
                e
            )));
        }

        // Phase 1: download + verify via fastembed.
        let show_progress = atty::is(atty::Stream::Stdout);
        let warmup = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGEM3)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(show_progress),
        )
        .map_err(|e| {
            CatClawError::Memory(format!("failed to init embedding model: {}", e))
        })?;
        // Drop immediately — frees the mmap-backed session. The on-disk
        // files are still cached and ready for phase 2.
        drop(warmup);

        // Phase 2: owned-bytes rebuild.
        let model = match load_bgem3_owned(&cache_dir) {
            Ok(m) => {
                info!(
                    "memory palace: BGE-M3 loaded as owned bytes (mmap-free, RSS +~2.3 GiB but no page-cache thrash)"
                );
                m
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "memory palace: owned-bytes load failed, falling back to mmap (model still works but may cause disk thrash on RAM-constrained hosts)"
                );
                // Re-init via try_new since we dropped the previous instance.
                TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::BGEM3)
                        .with_cache_dir(cache_dir)
                        .with_show_download_progress(false),
                )
                .map_err(|e| {
                    CatClawError::Memory(format!(
                        "failed to re-init embedding model after owned-bytes fallback: {}",
                        e
                    ))
                })?
            }
        };

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

/// Build a `TextEmbedding` whose weights are owned anonymous memory rather
/// than mmap-backed page cache. Reads everything fresh on each gateway
/// startup — fine because the buffers stay alive for the process lifetime
/// inside the `TextEmbedding`.
fn load_bgem3_owned(cache_dir: &Path) -> Result<TextEmbedding> {
    let snapshot = locate_bgem3_snapshot(cache_dir)?;

    // Main graph (small, ~17 MiB).
    let onnx_path = snapshot.join(BGEM3_ONNX_RELATIVE);
    let onnx_file = read_file(&onnx_path)?;

    // The two external initializers. `model.onnx_data` is the ~2.27 GiB
    // weight blob — the whole point of this dance. The constant attr is
    // tiny (~64 KiB) but the ONNX graph references it by file name, so
    // skipping it would either fail loading or trigger an mmap fallback.
    //
    // The `file_name` we register must match the basename the ONNX graph
    // expects — i.e. the literal string in the model's external-data
    // reference, NOT the path on disk.
    let onnx_dir = snapshot.join("onnx");
    let mut user_model =
        UserDefinedEmbeddingModel::new(onnx_file, load_bgem3_tokenizer(&snapshot)?)
            .with_pooling(Pooling::Cls);
    for name in BGEM3_ONNX_EXTERNAL_FILES {
        let buf = read_file(&onnx_dir.join(name))?;
        user_model = user_model.with_external_initializer(name.to_string(), buf);
    }

    TextEmbedding::try_new_from_user_defined(user_model, InitOptionsUserDefined::new())
        .map_err(|e| CatClawError::Memory(format!("try_new_from_user_defined failed: {}", e)))
}

fn load_bgem3_tokenizer(snapshot_dir: &Path) -> Result<TokenizerFiles> {
    Ok(TokenizerFiles {
        tokenizer_file: read_file(&snapshot_dir.join("tokenizer.json"))?,
        config_file: read_file(&snapshot_dir.join("config.json"))?,
        special_tokens_map_file: read_file(&snapshot_dir.join("special_tokens_map.json"))?,
        tokenizer_config_file: read_file(&snapshot_dir.join("tokenizer_config.json"))?,
    })
}

/// Resolve the active snapshot directory inside fastembed's hf-hub cache.
///
/// Layout (controlled by hf-hub crate, used internally by fastembed):
///   {cache_dir}/models--BAAI--bge-m3/refs/main          # one-line: commit hash
///   {cache_dir}/models--BAAI--bge-m3/snapshots/<hash>/  # the files (symlinks to blobs/)
fn locate_bgem3_snapshot(cache_dir: &Path) -> Result<PathBuf> {
    let repo_dir = cache_dir.join("models--BAAI--bge-m3");
    let refs_main = repo_dir.join("refs").join("main");
    let snapshot_hash = std::fs::read_to_string(&refs_main)
        .map_err(|e| {
            CatClawError::Memory(format!(
                "failed to read {} (expected fastembed cache layout): {}",
                refs_main.display(),
                e
            ))
        })?
        .trim()
        .to_string();
    if snapshot_hash.is_empty() {
        return Err(CatClawError::Memory(format!(
            "fastembed cache ref empty at {}",
            refs_main.display()
        )));
    }
    let snapshot_dir = repo_dir.join("snapshots").join(&snapshot_hash);
    if !snapshot_dir.is_dir() {
        return Err(CatClawError::Memory(format!(
            "fastembed snapshot dir missing: {}",
            snapshot_dir.display()
        )));
    }
    Ok(snapshot_dir)
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| CatClawError::Memory(format!("read {} failed: {}", path.display(), e)))
}
