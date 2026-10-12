//! Runtime backend abstraction (ADR-003). Backends plug into the runtime so
//! CPU/GPU/NPU/etc. can be swapped; aios-core itself stays dependency-free.

use crate::error::Result;

/// Compute backend that can load a GGUF model and run generation.
///
/// Implementations live in separate crates (e.g. `ai-inference` with Candle).
/// Trait is intentionally minimal: it must stay implementable by very small
/// custom backends (Rust-only SLM later) as well as heavy ones.
pub trait ComputeBackend {
    /// Short backend name, e.g. `"candle-cpu"`.
    fn name(&self) -> &'static str;

    /// Human-readable device summary, e.g. `"CPU (host)"`.
    fn device(&self) -> &'static str;

    /// Load a model file (GGUF) for generation.
    fn load_model(&mut self, path: &str) -> Result<()>;

    /// True once `load_model` succeeded.
    fn is_loaded(&self) -> bool;

    /// Generate a completion for `prompt`. Backends may cap tokens.
    fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String>;

    /// Streaming generation: yields tokens one at a time via iterator.
    /// Each item is a newly generated token string (may be partial word).
    /// Takes ownership of the backend; the returned iterator owns all generation state.
    fn generate_stream(
        self,
        prompt: &str,
        max_tokens: usize,
    ) -> Result<Box<dyn Iterator<Item = String> + Send + 'static>>;

    /// Measured throughput from the last load/generate window (tokens/s),
    /// or 0.0 when unknown. Non-fatal.
    fn tokens_per_second(&self) -> f64 {
        0.0
    }
}
