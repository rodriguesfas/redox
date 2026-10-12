//! Candle CPU backend (ADR-003). Loads Llama-architecture quantized GGUFs
//! through candle-transformers `quantized_llama` and runs greedy generation
//! on the Candle CPU device. Tokenizer is a companion `tokenizer.json`.

use aios_core::{ComputeBackend, Error, Result};
use candle_core::{quantized::gguf_file, DType, Device, Tensor};
use candle_transformers::models::quantized_llama as qllama;
use std::path::Path;
use std::time::Instant;

/// Companion tokenizer file resolvers, tried in order relative to the GGUF.
const TOKENIZER_CANDIDATES: [&str; 2] = ["{dir}/{stem}.tokenizer.json", "{dir}/tokenizer.json"];

pub struct CandleBackend {
    device: Device,
    model_name: String,
    model_path: String,
    llama: Option<qllama::ModelWeights>,
    tokenizer: Option<tokenizers::Tokenizer>,
    last_tps: f64,
    load_time: std::time::Duration,
}

fn resolve_tokenizer(model_path: &Path) -> Option<std::path::PathBuf> {
    // `Path::parent` on a bare filename ("model.gguf") returns Some(""), not
    // None, and the templates below are built by string interpolation. An empty
    // dir therefore yields "/tokenizer.json" -- an absolute path at the
    // filesystem root -- so a sibling tokenizer is never found and the model
    // is reported as having no tokenizer at all. Only a path that actually
    // carries a directory is usable here.
    let dir = match model_path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let stem = model_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model");
    for tmpl in TOKENIZER_CANDIDATES {
        let candidate = tmpl
            .replace("{dir}", &dir.display().to_string())
            .replace("{stem}", stem);
        let p = Path::new(&candidate);
        if p.is_file() {
            return Some(p.to_owned());
        }
    }
    // Fallback: a single `*.tokenizer.json` in the directory is unambiguous
    // even when its stem does not match the GGUF (e.g. one tokenizer file
    // shared by quantized variants downloaded under different names).
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut matches: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(".tokenizer.json"))
                        .unwrap_or(false)
            })
            .collect();
        if matches.len() == 1 {
            return matches.pop();
        }
    }
    None
}

impl CandleBackend {
    pub fn new() -> Result<Self> {
        let device = Device::Cpu;
        Ok(Self {
            device,
            model_name: String::new(),
            model_path: String::new(),
            llama: None,
            tokenizer: None,
            last_tps: 0.0,
            load_time: std::time::Duration::ZERO,
        })
    }
}

impl ComputeBackend for CandleBackend {
    fn name(&self) -> &'static str {
        "candle-cpu"
    }

    fn device(&self) -> &'static str {
        "CPU (Candle core)"
    }

    fn load_model(&mut self, path: &str) -> Result<()> {
        let path = Path::new(path);
        let tokenizer_path = resolve_tokenizer(path).ok_or_else(|| {
            Error::Msg(format!(
                "no tokenizer.json found next to {} (expected {}.tokenizer.json or tokenizer.json in the same dir)",
                path.display(),
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("model")
            ))
        })?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::Msg(format!("tokenizer: {e}")))?;

        let start = Instant::now();
        let mut file = std::fs::File::open(path)?;
        let content =
            gguf_file::Content::read(&mut file).map_err(|e| Error::Msg(format!("gguf: {e}")))?;
        let llama = qllama::ModelWeights::from_gguf(content, &mut file, &self.device)
            .map_err(|e| Error::Msg(format!("llama: {e}")))?;
        self.load_time = start.elapsed();

        self.model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_string();
        self.model_path = path.display().to_string();
        self.llama = Some(llama);
        self.tokenizer = Some(tokenizer);
        self.last_tps = 0.0;
        Ok(())
    }

    fn is_loaded(&self) -> bool {
        self.llama.is_some() && self.tokenizer.is_some()
    }

    fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String> {
        let llama = self
            .llama
            .as_mut()
            .ok_or_else(|| Error::Msg("no model loaded".into()))?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| Error::Msg("no tokenizer loaded".into()))?;

        let prompt_ids = tokenizer
            .encode(prompt, true)
            .map_err(|e| Error::Msg(format!("encode: {e}")))?
            .get_ids()
            .to_vec();
        let mut all: Vec<u32> = prompt_ids;
        let eos = tokenizer.token_to_id("</s>");
        let start = Instant::now();

        let mut out = String::new();
        let mut index_pos = 0usize;
        let mut generated = 0usize;
        for i in 0..max_tokens.max(1) {
            let context = if i > 0 { 1 } else { all.len() };
            let start_pos = all.len().saturating_sub(context);
            let ctxt = &all[start_pos..all.len()];
            let input = Tensor::new(ctxt, &self.device)
                .map_err(|e| Error::Msg(format!("tensor: {e}")))?
                .unsqueeze(0)
                .map_err(|e| Error::Msg(format!("unsqueeze: {e}")))?;
            let logits = llama
                .forward(&input, index_pos)
                .map_err(|e| Error::Msg(format!("forward: {e}")))?
                .squeeze(0)
                .map_err(|e| Error::Msg(format!("squeeze: {e}")))?
                .to_dtype(DType::F32)
                .map_err(|e| Error::Msg(format!("dtype: {e}")))?;
            let next_token = logits
                .argmax(0)
                .map_err(|e| Error::Msg(format!("argmax: {e}")))?
                .to_scalar::<u32>()
                .map_err(|e| Error::Msg(format!("scalar: {e}")))?;
            index_pos += ctxt.len();

            let prev_len = all.len();
            all.push(next_token);
            generated += 1;
            let fresh = &all[prev_len..all.len()];
            let text = tokenizer
                .decode(fresh, true)
                .map_err(|e| Error::Msg(format!("decode: {e}")))?;
            out.push_str(&text);

            if let Some(eos) = eos {
                if next_token == eos {
                    break;
                }
            }
        }
        self.last_tps = generated as f64 / start.elapsed().as_secs_f64();
        Ok(out)
    }

    fn generate_stream(
        mut self,
        prompt: &str,
        max_tokens: usize,
    ) -> Result<Box<dyn Iterator<Item = String> + Send + 'static>> {
        if self.llama.is_none() || self.tokenizer.is_none() {
            return Err(Error::Msg("no model loaded".into()));
        }

        let prompt_ids = self
            .tokenizer
            .as_ref()
            .unwrap()
            .encode(prompt, true)
            .map_err(|e| Error::Msg(format!("encode: {e}")))?
            .get_ids()
            .to_vec();
        let mut all = prompt_ids;
        let eos = self.tokenizer.as_ref().unwrap().token_to_id("</s>");
        let mut index_pos = 0usize;
        let mut generated = 0usize;

        Ok(Box::new(std::iter::from_fn(move || {
            if generated >= max_tokens.max(1) {
                return None;
            }
            let context = if generated > 0 { 1 } else { all.len() };
            let start_pos = all.len().saturating_sub(context);
            let ctxt = &all[start_pos..all.len()];
            let input = Tensor::new(ctxt, &self.device).ok()?.unsqueeze(0).ok()?;
            let llama = self.llama.as_mut()?;
            let logits = llama.forward(&input, index_pos).ok()?;
            let logits = logits.squeeze(0).ok()?.to_dtype(DType::F32).ok()?;
            let next_token = logits.argmax(0).ok()?.to_scalar::<u32>().ok()?;
            index_pos += ctxt.len();

            let prev_len = all.len();
            all.push(next_token);
            generated += 1;
            let fresh = &all[prev_len..all.len()];
            let text = self.tokenizer.as_ref().unwrap().decode(fresh, true).ok()?;

            if let Some(eos) = eos {
                if next_token == eos {
                    // tps is not recorded here: the stream consumes `self`, so
                    // there is no backend left for callers to read it from.
                    return None;
                }
            }
            Some(text)
        })))
    }

    fn tokens_per_second(&self) -> f64 {
        self.last_tps
    }
}

impl Default for CandleBackend {
    fn default() -> Self {
        CandleBackend::new().unwrap()
    }
}

/// Load time for the current model (last `load_model` call).
pub fn load_time(backend: &CandleBackend) -> std::time::Duration {
    backend.load_time
}

/// Model file currently loaded.
pub fn model_path(backend: &CandleBackend) -> &str {
    &backend.model_path
}

/// Model name (GGUF file stem) currently loaded.
pub fn model_name(backend: &CandleBackend) -> &str {
    &backend.model_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tokenizer_finds_sibling_for_bare_filename() {
        let dir = std::env::temp_dir().join(format!("aios-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stem = "bare-name-test";
        std::fs::write(dir.join(format!("{stem}.tokenizer.json")), "{}").unwrap();

        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let found = resolve_tokenizer(Path::new(&format!("{stem}.gguf")))
            .map(|p| p.canonicalize().unwrap());
        std::env::set_current_dir(prev).unwrap();

        assert_eq!(
            found,
            Some(
                dir.join(format!("{stem}.tokenizer.json"))
                    .canonicalize()
                    .unwrap()
            ),
            "tokenizer next to a bare filename must resolve in the cwd"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_tokenizer_falls_back_to_single_suffixed_file() {
        let dir = std::env::temp_dir().join(format!("aios-tok3-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok(); // clean leftovers from failed runs
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("other.tokenizer.json"), "{}").unwrap();

        let found = resolve_tokenizer(&dir.join("model.gguf"));
        assert_eq!(
            found.as_deref(),
            Some(dir.join("other.tokenizer.json").as_path()),
            "a single *.tokenizer.json resolves regardless of stem"
        );

        // Two candidates with unrelated stems are ambiguous: no fallback.
        std::fs::write(dir.join("second.tokenizer.json"), "{}").unwrap();
        assert!(resolve_tokenizer(&dir.join("model.gguf")).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_tokenizer_prefers_stem_specific_file() {
        let dir = std::env::temp_dir().join(format!("aios-tok2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.tokenizer.json"), "{}").unwrap();
        std::fs::write(dir.join("tokenizer.json"), "{}").unwrap();

        let found = resolve_tokenizer(&dir.join("model.gguf"));
        assert_eq!(
            found.as_deref(),
            Some(dir.join("model.tokenizer.json").as_path())
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
