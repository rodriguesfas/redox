//! Daemon runtime: model resolution + a single-model inference cache, request
//! statistics, a bounded log ring and a metrics history ring.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use aios_core::{cache_path_for, default_models_dir, list_installed, ModelMeta, Registry};
use aios_inference::{CandleBackend, ComputeBackend};

use crate::metrics::Sample;

pub const MAX_LOGS: usize = 2000;
pub const MAX_HISTORY: usize = 240;

#[derive(Debug, Clone, serde::Serialize)]
pub struct LogLine {
    pub ts: f64,
    pub level: String,
    pub msg: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct InferStats {
    pub requests: u64,
    pub errors: u64,
    pub tokens: u64,
    pub total_ms: u128,
    pub last_tps: f64,
}

impl InferStats {
    pub fn avg_latency_ms(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.total_ms as f64 / self.requests as f64
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryPoint {
    pub ts: f64,
    pub cpu_percent: Option<f64>,
    pub mem_used_mb: Option<f64>,
    pub requests: u64,
    pub errors: u64,
    pub tokens: u64,
    pub last_tps: f64,
}

/// Outcome of an inference request.
pub struct InferOut {
    pub text: String,
    pub tps: f64,
    pub load_ms: u128,
    pub cached: bool,
}

struct Cache {
    backend: CandleBackend,
    path: String,
}

pub struct Runtime {
    start: Instant,
    logs: Mutex<VecDeque<LogLine>>,
    model_cache: Mutex<Option<Cache>>,
    stats: Mutex<InferStats>,
    history: Mutex<VecDeque<HistoryPoint>>,
    last_sample: Mutex<Option<Sample>>,
}

fn now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl Runtime {
    pub fn new() -> Runtime {
        Runtime {
            start: Instant::now(),
            logs: Mutex::new(VecDeque::new()),
            model_cache: Mutex::new(None),
            stats: Mutex::new(InferStats::default()),
            history: Mutex::new(VecDeque::new()),
            last_sample: Mutex::new(None),
        }
    }

    pub fn uptime_s(&self) -> u64 {
        self.start.elapsed().as_secs()
    }

    pub fn log(&self, level: &str, msg: impl AsRef<str>) {
        let mut logs = self.logs.lock().unwrap();
        if logs.len() == MAX_LOGS {
            logs.pop_front();
        }
        logs.push_back(LogLine {
            ts: now(),
            level: level.to_string(),
            msg: msg.as_ref().to_string(),
        });
    }

    pub fn logs(&self) -> Vec<LogLine> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }

    pub fn stats(&self) -> InferStats {
        self.stats.lock().unwrap().clone()
    }

    fn record(&self, err: bool, tokens: usize, wall_ms: u128, tps: f64) {
        let mut s = self.stats.lock().unwrap();
        s.requests += 1;
        if err {
            s.errors += 1;
        }
        s.tokens += tokens as u64;
        s.total_ms += wall_ms;
        if !err {
            s.last_tps = tps;
        }
    }

    /// Push a periodic observation into the history ring and keep it as the
    /// latest system sample.
    pub fn push_history(&self, sample: &Sample) {
        let s = self.stats.lock().unwrap().clone();
        let mut h = self.history.lock().unwrap();
        if h.len() == MAX_HISTORY {
            h.pop_front();
        }
        h.push_back(HistoryPoint {
            ts: now(),
            cpu_percent: sample.cpu_percent,
            mem_used_mb: sample.mem_used_mb,
            requests: s.requests,
            errors: s.errors,
            tokens: s.tokens,
            last_tps: s.last_tps,
        });
        *self.last_sample.lock().unwrap() = Some(sample.clone());
    }

    /// Latest system sample collected by the daemon sampler thread.
    pub fn last_sample(&self) -> Option<Sample> {
        self.last_sample.lock().unwrap().clone()
    }

    pub fn history(&self) -> Vec<HistoryPoint> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    /// Resolve a model reference to a local path: file -> registry -> cache.
    pub fn resolve_model(&self, name: &str) -> Result<String, String> {
        if Path::new(name).is_file() {
            return Ok(name.to_string());
        }
        let reg = Registry::load(aios_core::default_registry_file())
            .map_err(|e| format!("registry: {e}"))?;
        if let Some(e) = reg.find(name) {
            return Ok(e.path.clone());
        }
        let guess = cache_path_for(Path::new(&default_models_dir()), name);
        if guess.is_file() {
            return Ok(guess.display().to_string());
        }
        Err(format!(
            "model '{name}' not found as a file, in the registry or in the cache"
        ))
    }

    /// List installed models (path, size, GGUF arch if parseable).
    pub fn list_models(&self) -> Result<Vec<ModelMeta>, String> {
        list_installed(Path::new(&default_models_dir())).map_err(|e| e.to_string())
    }

    /// Register a model in the runtime's cache and aios-core registry.
    pub fn register_model(&self, entry: aios_core::RegistryEntry) -> Result<(), String> {
        // Add to aios-core registry
        let mut reg = Registry::load(aios_core::default_registry_file())
            .map_err(|e| format!("registry: {e}"))?;
        reg.add(entry.clone());
        reg.save().map_err(|e| format!("registry save: {e}"))?;

        // Invalidate cache so next infer loads the new model
        *self.model_cache.lock().unwrap() = None;
        Ok(())
    }

    /// Run inference, caching one model at a time across requests.
    pub fn infer(&self, model: &str, prompt: &str, max_tokens: usize) -> Result<InferOut, String> {
        let wall = Instant::now();
        let path = self.resolve_model(model).inspect_err(|_| {
            self.record(true, 0, 0, 0.0);
        })?;
        let result = self.infer_path(&path, prompt, max_tokens);
        match &result {
            Ok(out) => self.record(false, max_tokens, wall.elapsed().as_millis(), out.tps),
            Err(e) => {
                self.log("error", format!("infer {path}: {e}"));
                self.record(true, max_tokens, wall.elapsed().as_millis(), 0.0);
            }
        }
        result
    }

    /// Run streaming inference. Creates a dedicated backend for the stream
    /// (the shared cache stays untouched for non-streaming requests).
    /// Returns an iterator that yields tokens as they are generated.
    pub fn infer_stream(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: usize,
    ) -> Result<Box<dyn Iterator<Item = String> + Send + 'static>, String> {
        let path = self.resolve_model(model).inspect_err(|_| {
            self.record(true, 0, 0, 0.0);
        })?;

        // For streaming, create a dedicated local backend — the returned
        // iterator consumes it, so the shared cache stays free for infer().
        let mut backend = CandleBackend::new().map_err(|e| format!("backend: {e}"))?;
        backend
            .load_model(&path)
            .map_err(|e| format!("load {path}: {e}"))?;

        backend
            .generate_stream(prompt, max_tokens.max(1))
            .map_err(|e| format!("generate_stream: {e}"))
    }

    fn infer_path(&self, path: &str, prompt: &str, max_tokens: usize) -> Result<InferOut, String> {
        let load_ms = self.load_only(path)?;
        let mut cache = self.model_cache.lock().unwrap();
        let c = cache.as_mut().unwrap();
        let text = c
            .backend
            .generate(prompt, max_tokens.max(1))
            .map_err(|e| format!("generate: {e}"))?;
        let tps = c.backend.tokens_per_second();
        Ok(InferOut {
            text,
            tps,
            load_ms,
            cached: load_ms == 0,
        })
    }

    /// Load `path` into the cache if it is not already the cached model.
    /// Returns the load time in ms (0 if it was already cached).
    pub fn load_only(&self, path: &str) -> Result<u128, String> {
        let mut cache = self.model_cache.lock().unwrap();
        if let Some(c) = cache.as_ref() {
            if c.path == path && c.backend.is_loaded() {
                return Ok(0);
            }
        }
        let mut backend = CandleBackend::new().map_err(|e| format!("backend: {e}"))?;
        backend
            .load_model(path)
            .map_err(|e| format!("load {path}: {e}"))?;
        let load_ms = aios_inference::load_time(&backend).as_millis();
        *cache = Some(Cache {
            backend,
            path: path.to_string(),
        });
        Ok(load_ms)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::new()
    }
}

/// Preload a model into the cache at daemon startup (optional).
pub fn preload(runtime: &Runtime, model: &str) -> Result<(), String> {
    let path = runtime.resolve_model(model)?;
    let load_ms = runtime.load_only(&path)?;
    runtime.log("info", format!("preloaded {model} (load {load_ms} ms)"));
    Ok(())
}
