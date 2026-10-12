//! AIOS edge SDK: exposes `ai::load` / `model.generate`-style API for edge
//! applications. Thin facade over aios-core + aios-inference. `load` resolves
//! a model from the local registry/cache (Fase 3); `generate` runs real
//! inference through the Candle CPU backend (Fase 4).

use std::fs;
use std::path::Path;

pub use aios_core::*;

/// A model resolved by [`load`]: path, size and integrity info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedModel {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    /// Whether the model is tracked in the registry (installed via `ai install`).
    pub registered: bool,
}

/// Resolve a model by name: prefers the registry, falls back to a plain
/// `<cache>/<name>.gguf` file. Fails if neither exists (or the registry
/// cannot be read).
pub fn load(name: &str) -> aios_core::Result<LoadedModel> {
    let reg = Registry::load(default_registry_file())?;
    if let Some(e) = reg.find(name) {
        if fs::metadata(&e.path).is_ok() {
            return Ok(LoadedModel {
                name: name.to_string(),
                path: e.path.clone(),
                size_bytes: e.size_bytes,
                sha256: e.sha256.clone(),
                registered: true,
            });
        }
        return Err(Error::Msg(format!(
            "model '{name}' is registered but missing on disk: {}",
            e.path
        )));
    }

    let guess = cache_path_for(Path::new(&default_models_dir()), name);
    if guess.is_file() {
        let size_bytes = fs::metadata(&guess)?.len();
        return Ok(LoadedModel {
            name: name.to_string(),
            path: guess.display().to_string(),
            size_bytes,
            sha256: String::new(),
            registered: false,
        });
    }

    Err(Error::Msg(format!(
        "model '{name}' not found in registry or in {}",
        guess.display()
    )))
}

/// Result of a real inference run, with timing/throughput metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct Generation {
    /// Decoded output text (stops at EOS or `max_tokens`).
    pub text: String,
    /// Generated tokens per second (wall time of the generation loop).
    pub tokens_per_second: f64,
    /// Model load time (GGUF + weights + tokenizer).
    pub load_ms: u128,
}

/// Generate a completion for `prompt` through the Candle CPU backend.
///
/// Loads the model file pointed to by [`load`], runs greedy generation and
/// returns the decoded text. Uses a default budget of 64 generated tokens;
/// use [`generate_with_metrics`] for full control and metrics.
pub fn generate(loaded: &LoadedModel, prompt: &str) -> aios_core::Result<String> {
    generate_with_metrics(loaded, prompt, 64).map(|g| g.text)
}

/// Like [`generate`] but with an explicit token budget and metrics
/// (tokens/s, load time).
pub fn generate_with_metrics(
    loaded: &LoadedModel,
    prompt: &str,
    max_tokens: usize,
) -> aios_core::Result<Generation> {
    let mut backend =
        aios_inference::CandleBackend::new().map_err(|e| Error::Msg(format!("backend: {e}")))?;
    backend
        .load_model(&loaded.path)
        .map_err(|e| Error::Msg(format!("load {}: {e}", loaded.path)))?;
    let load_ms = aios_inference::load_time(&backend).as_millis();
    let text = backend
        .generate(prompt, max_tokens)
        .map_err(|e| Error::Msg(format!("generate: {e}")))?;
    let tokens_per_second = backend.tokens_per_second();
    Ok(Generation {
        text,
        tokens_per_second,
        load_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// `load` reads its configuration from process-wide environment variables,
    /// so these tests cannot run concurrently: whichever test set the variables
    /// last wins for everyone. The lock serializes them, which is cheap for
    /// four tests and does not depend on scheduling luck.
    ///
    /// This was a real flake, not a hypothetical one. The tests used to share
    /// a directory derived only from the process id and wipe it on setup, so a
    /// parallel `setup()` deleted another test's fixture and its `AIOS_MODELS_DIR`
    /// pointed somewhere the model was not. It passed in CI for several runs and
    /// then failed on `load_via_cache_file` with "model 'tiny' not found".
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let m = LOCK.get_or_init(|| Mutex::new(()));
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn gguf_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GGUF");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b
    }

    /// A directory unique to this test, not just to this process. Callers hold
    /// the environment lock for the whole test, so the name only has to be
    /// stable and collision-free.
    fn setup(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aios-sdk-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_via_registry() {
        let _guard = env_lock();
        let dir = setup("registry");
        let cache = dir.join("models");
        fs::create_dir_all(&cache).unwrap();
        let src = cache.join("m.gguf");
        fs::write(&src, gguf_bytes()).unwrap();

        let reg_path = dir.join("registry.tsv");
        let entry = aios_core::install_model(&src, &cache, "demo").unwrap();
        let mut reg = aios_core::Registry::load(reg_path.as_path()).unwrap();
        reg.add(entry);
        reg.save().unwrap();

        std::env::set_var("AIOS_REGISTRY", reg_path.as_path());
        std::env::set_var("AIOS_MODELS_DIR", cache.as_path());

        let m = load("demo").expect("load via registry");
        assert_eq!(m.name, "demo");
        assert!(m.registered);
        assert_eq!(m.size_bytes, 24);
        assert_eq!(m.sha256.len(), 64);
    }

    #[test]
    fn load_via_cache_file() {
        let _guard = env_lock();
        let dir = setup("cache-file");
        let cache = dir.join("models");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("tiny.gguf"), gguf_bytes()).unwrap();

        std::env::set_var("AIOS_MODELS_DIR", cache.as_path());
        std::env::set_var("AIOS_REGISTRY", dir.join("empty.tsv").as_path());

        let m = load("tiny").expect("load via cache file");
        assert_eq!(m.name, "tiny");
        assert!(!m.registered);
        assert_eq!(m.sha256, "");
    }

    #[test]
    fn load_missing_model_fails() {
        let _guard = env_lock();
        let dir = setup("missing");
        std::env::set_var("AIOS_MODELS_DIR", dir.join("models").as_path());
        std::env::set_var("AIOS_REGISTRY", dir.join("empty.tsv").as_path());
        assert!(load("nope").is_err());
    }

    #[test]
    fn generate_missing_model_fails() {
        let m = LoadedModel {
            name: "x".into(),
            path: "/nonexistent/does-not-exist.gguf".into(),
            size_bytes: 0,
            sha256: String::new(),
            registered: false,
        };
        assert!(generate(&m, "hi").is_err());
    }
}
