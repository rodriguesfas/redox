//! ai-inference: AIOS inference backends (ADR-003).
//!
//! First backend: Candle CPU (`candle-cpu`), loading Llama-arch quantized
//! GGUF models via candle-transformers and running greedy generation with a
//! `tokenizers` companion file. Cross-compiles cleanly for the
//! `*-unknown-redox` targets.

pub mod candle_backend;

pub use aios_core::ComputeBackend;
pub use candle_backend::{load_time, model_name, model_path, CandleBackend};
