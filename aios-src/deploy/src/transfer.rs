//! Model Transfer for AIOS Edge Deployment
//!
//! HTTP-based model transfer with retry and checksum verification.

use aios_core::{checksum, ModelMeta};
use anyhow::{Context, Result};
use reqwest::blocking::Client;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TransferConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    /// Retries *after* the first attempt, so the total number of attempts is
    /// `max_retries + 1`. The old loop ran `1..=max_retries`, which meant a
    /// value of 3 gave three attempts in total and the field name lied about
    /// it.
    pub max_retries: u32,
    pub chunk_size: usize,
    /// Re-read the model from disk after a successful upload and confirm its
    /// SHA-256 still matches the one we sent, which catches a file that changed
    /// underneath the transfer.
    ///
    /// This is *not* a weaker version of the device's own check: the edge
    /// daemon recomputes the digest of the bytes it reassembled and rejects a
    /// mismatch regardless of this flag. `--no-verify` cannot make a corrupt
    /// model install.
    pub verify_checksum: bool,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8989".to_string(),
            timeout_secs: 300,
            max_retries: 3,
            chunk_size: 1024 * 1024, // 1MB chunks
            verify_checksum: true,
        }
    }
}

/// Transfer a model to an edge device via HTTP
pub fn transfer_model(
    config: &TransferConfig,
    _device_id: &str,
    model_path: &Path,
    model_name: &str,
) -> Result<TransferResult> {
    let client = Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build()
        .context("building HTTP client")?;

    if config.chunk_size == 0 {
        return Err(anyhow::anyhow!("chunk_size must be greater than zero"));
    }

    // Hold the file open for the whole transfer and stream it. The previous
    // implementation did `fs::read`, so peak memory was the model size plus a
    // copy of every chunk — a 4 GB model could not be deployed on the very
    // edge devices this crate targets, which have 1-4 GB of RAM.
    let mut file = fs::File::open(model_path)
        .with_context(|| format!("opening model file {}", model_path.display()))?;
    let model_size = file
        .metadata()
        .with_context(|| format!("reading size of {}", model_path.display()))?
        .len();
    if model_size == 0 {
        return Err(anyhow::anyhow!(
            "model file {} is empty; nothing to deploy",
            model_path.display()
        ));
    }

    // Parse metadata before pushing anything: a model we cannot read is better
    // rejected up front than after half of it is on the wire. This also runs
    // the GGUF parser, whose bounds checks are what stop a corrupt file from
    // ever reaching the upload path.
    ModelMeta::from_path(model_path).map_err(|e| {
        anyhow::anyhow!(
            "reading model metadata from {}: {}",
            model_path.display(),
            e
        )
    })?;

    // The protocol requires the digest, so it is always computed. Hashing
    // streams from the file rather than from a buffer, which is the only
    // reason a multi-GB model can be hashed without a multi-GB allocation.
    let sha256 = checksum::sha256_hex(&mut file)
        .with_context(|| format!("computing SHA-256 of {}", model_path.display()))?;
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("rewinding {}", model_path.display()))?;

    let base_url = config.base_url.trim_end_matches('/');
    let max_attempts = config.max_retries.saturating_add(1);
    let mut last_error = None;

    for attempt in 1..=max_attempts {
        match upload_chunked(
            &client,
            base_url,
            &mut file,
            model_size,
            config.chunk_size,
            model_name,
            &sha256,
        ) {
            Ok(()) => {
                if config.verify_checksum {
                    // Confirm the file we hashed is still the file we sent. A
                    // model replaced mid-transfer would otherwise install with
                    // a digest that no longer describes anything on disk.
                    let mut f = fs::File::open(model_path).with_context(|| {
                        format!("re-opening {} to verify the transfer", model_path.display())
                    })?;
                    let after = checksum::sha256_hex(&mut f).with_context(|| {
                        format!("re-reading {} to verify the transfer", model_path.display())
                    })?;
                    if after != sha256 {
                        return Err(anyhow::anyhow!(
                            "model file {} changed while it was being deployed \
                             (sent {sha256}, now {after}); re-run the deploy",
                            model_path.display()
                        ));
                    }
                }
                return Ok(TransferResult {
                    success: true,
                    model_name: model_name.to_string(),
                    model_size,
                    sha256,
                    bytes_transferred: model_size,
                    attempts: attempt,
                    error: None,
                });
            }
            Err(e) => {
                last_error = Some(e.to_string());
                eprintln!("Upload attempt {attempt}/{max_attempts} failed: {e}");
                if attempt < max_attempts {
                    // Exponential backoff. The original code slept a flat 2s,
                    // so a device that was down cost 2s per retry with no
                    // widening; a device that was merely congested got hit
                    // again at the same instant.
                    let backoff = retry_backoff(attempt);
                    eprintln!("  retrying in {backoff:?}");
                    std::thread::sleep(backoff);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Transfer failed after {max_attempts} attempts: {}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

/// Backoff before retry `attempt` (1-based): 2s, 4s, 8s, capped at 30s.
fn retry_backoff(attempt: u32) -> Duration {
    let secs = 2u32.saturating_pow(attempt.saturating_sub(1)).min(30);
    Duration::from_secs(secs as u64)
}

/// Transfer result summary
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransferResult {
    pub success: bool,
    pub model_name: String,
    pub model_size: u64,
    pub sha256: String,
    pub bytes_transferred: u64,
    pub attempts: u32,
    pub error: Option<String>,
}

/// Upload model in chunks via multipart
///
/// Reads from `file` chunk by chunk, holding at most one chunk in memory.
/// Rewinds to the start on entry so a retry after a mid-transfer failure
/// resends the whole model rather than the tail of the previous attempt.
fn upload_chunked(
    client: &Client,
    base_url: &str,
    file: &mut fs::File,
    model_size: u64,
    chunk_size: usize,
    model_name: &str,
    sha256: &str,
) -> Result<()> {
    let total_chunks = model_size.div_ceil(chunk_size as u64);
    let upload_url = format!("{base_url}/api/models/upload");

    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("rewinding {model_name} to resend from the start"))?;

    let mut buf = vec![0u8; chunk_size];
    for chunk_idx in 0..total_chunks {
        let offset = chunk_idx * chunk_size as u64;
        let want = chunk_size.min((model_size - offset) as usize);
        file.read_exact(&mut buf[..want]).with_context(|| {
            format!("reading chunk {chunk_idx} of {model_name} (expected {want} bytes at offset {offset})")
        })?;

        let part = reqwest::blocking::multipart::Part::bytes(buf[..want].to_vec())
            .file_name(format!("{model_name}.part{chunk_idx}"));

        let form = reqwest::blocking::multipart::Form::new()
            .part("chunk", part)
            .text("model_name", model_name.to_string())
            .text("sha256", sha256.to_string())
            .text("chunk_index", chunk_idx.to_string())
            .text("total_chunks", total_chunks.to_string());

        let resp = client.post(&upload_url).multipart(form).send()?;

        if !resp.status().is_success() {
            let err_text = resp.text().unwrap_or_else(|_| "unknown error".to_string());
            return Err(anyhow::anyhow!(
                "Chunk {chunk_idx} upload failed: {err_text}"
            ));
        }
    }

    // Finalize. The daemon's route is a *nested* path,
    // "/api/models/upload/finalize", not a sibling of the chunk route. Building
    // it by appending "/finalize" to the upload URL happens to land on the
    // right path, but it reads as if it were a sibling and would silently break
    // if either route were ever reshaped. State it outright.
    let finalize_url = format!("{base_url}/api/models/upload/finalize");
    let resp = client
        .post(&finalize_url)
        .json(&serde_json::json!({
            "model_name": model_name,
            "sha256": sha256,
            "total_chunks": total_chunks
        }))
        .send()?;

    if !resp.status().is_success() {
        let err_text = resp.text().unwrap_or_else(|_| "unknown error".to_string());
        // Surface the device's own words. The daemon answers a checksum
        // mismatch with "sha256 mismatch: expected X got Y", which is the
        // single most useful thing to tell an operator — without it the error
        // is just "Finalize failed" and looks like any other network error.
        return Err(anyhow::anyhow!("Finalize failed: {err_text}"));
    }

    Ok(())
}

/// What a device reported about itself and the freshly deployed model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthReport {
    /// The `status` field from the device's health response.
    pub status: String,
    /// The `service` field, expected to be "edge-ai".
    pub service: String,
    /// The daemon's version string, if it reported one.
    pub version: String,
    /// The model this check was for, carried through for the report.
    pub model: String,
    /// Whether the device's model list contained the deployed model.
    pub model_present: bool,
}

/// Check that a device is serving after a deploy.
///
/// Two requests: `GET /api/health` for liveness, then `GET /api/models` to
/// confirm the model is actually loaded rather than merely written to disk.
/// The second one is the part that matters — a daemon can be healthy with a
/// missing or unloadable model, and reporting that as a successful deployment
/// would be wrong.
pub fn health_check(base_url: &str, model_name: &str) -> Result<HealthReport> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("building health-check client")?;
    let base = base_url.trim_end_matches('/');

    let health: serde_json::Value = client
        .get(format!("{base}/api/health"))
        .send()
        .context("health request failed")?
        .error_for_status()
        .context("device reported an error on /api/health")?
        .json()
        .context("parsing /api/health response")?;

    let status = health["status"].as_str().unwrap_or("unknown").to_string();
    let service = health["service"].as_str().unwrap_or("unknown").to_string();
    let version = health["version"].as_str().unwrap_or("unknown").to_string();

    if status != "ok" {
        return Err(anyhow::anyhow!(
            "device health is '{status}', expected 'ok' (service: {service})"
        ));
    }

    // Confirm the model is loaded. A device that is up but cannot list models
    // is not a successful deployment.
    let models: serde_json::Value = client
        .get(format!("{base}/api/models"))
        .send()
        .context("model list request failed")?
        .error_for_status()
        .context("device reported an error on /api/models")?
        .json()
        .context("parsing /api/models response")?;

    let model_present = models
        .get("models")
        .and_then(|m| m.as_array())
        .map(|list| {
            list.iter().any(|entry| {
                entry
                    .get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n == model_name)
            })
        })
        .unwrap_or(false);

    if !model_present {
        let listed = models
            .get("models")
            .and_then(|m| m.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|e| e.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "<none>".to_string());
        return Err(anyhow::anyhow!(
            "device is healthy but does not list model '{model_name}' (has: {listed})"
        ));
    }

    Ok(HealthReport {
        status,
        service,
        version,
        model: model_name.to_string(),
        model_present,
    })
}

/// Pull model from edge device (for backup/sync)
pub fn pull_model(config: &TransferConfig, model_name: &str, output_path: &Path) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build()?;

    let url = format!(
        "{}/api/models/{}/download",
        config.base_url.trim_end_matches('/'),
        model_name
    );
    let mut resp = client.get(&url).send()?;

    if !resp.status().is_success() {
        let err_text = resp.text().unwrap_or_else(|_| "unknown error".to_string());
        return Err(anyhow::anyhow!("Download failed: {}", err_text));
    }

    let mut file = fs::File::create(output_path)
        .with_context(|| format!("creating output file {}", output_path.display()))?;

    let mut downloaded = 0u64;
    let mut buffer = vec![0u8; config.chunk_size];
    loop {
        let n = std::io::Read::read(&mut resp, &mut buffer)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..n])?;
        downloaded += n as u64;
    }

    println!(
        "Downloaded {} bytes to {}",
        downloaded,
        output_path.display()
    );
    Ok(())
}
