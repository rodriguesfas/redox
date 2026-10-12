//! Phase 7 end-to-end: a model transferred over HTTP with retry, checksum
//! verification and a post-deploy health check, against a server that speaks
//! the edge-ai daemon's actual protocol.
//!
//! Scope, stated plainly: the payload is a synthetic GGUF-shaped file, not a
//! trained SLM. This exercises the transport and install protocol, not model
//! loading or inference quality.
//!
//! The server here is a re-implementation of `edge_ai::api`'s upload
//! endpoints (`POST /api/models/upload` chunked multipart, then
//! `POST /api/models/upload/finalize`), assembled the same way: chunks are
//! keyed by (model_name, index), the count must reach total_chunks, and the
//! SHA-256 is verified before the file is accepted. Running the real daemon
//! instead would be better, but it resolves `AIOS_MODELS_DIR` and the registry
//! from process-global paths, which would make this test write outside its
//! sandbox. The protocol shape is what is under test, and it is asserted
//! rather than assumed — see `the_mock_speaks_the_daemons_protocol`.
//!
//! What this does NOT cover: the aarch64/QEMU or Raspberry Pi target itself.
//! That leg needs hardware this repo does not have, and the Redox NVMe hang
//! blocks the QEMU leg. See ROADMAP.md Phase 7.

use aios_core::checksum::sha256_hex_bytes;
use aios_deploy::{transfer_model, TransferConfig};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// What the fake edge device recorded.
#[derive(Default)]
struct Received {
    /// (model_name, chunk_index) -> bytes, as the daemon's UploadState does.
    chunks: HashMap<(String, usize), Vec<u8>>,
    /// Chunks whose body the server intentionally corrupted.
    corrupted: Vec<usize>,
    /// sha256 declared on the first chunk of each upload.
    declared_sha: HashMap<String, String>,
    /// Chunk requests that arrived at all, per index (retries included).
    requests_per_chunk: HashMap<usize, usize>,
    /// Models that passed finalize.
    installed: Vec<(String, String)>,
    /// Total chunk POSTs, to assert retry actually re-sent.
    total_posts: usize,
}

struct FakeDevice {
    port: u16,
    state: Arc<Mutex<Received>>,
    stop: Arc<AtomicBool>,
    /// Chunk indices to reject exactly once each.
    fail_once: Arc<Mutex<HashSet<usize>>>,
    /// Run after the device accepts chunk 0, i.e. after the client has hashed
    /// the file but before it has finished. Lets a test race the model file
    /// against its own transfer. Behind a lock so a test can install the hook
    /// after the device is already listening.
    on_chunk0: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    /// Kept so the listener thread's JoinHandle is not dropped, which would
    /// detach it silently. The thread exits when `stop` is set on Drop.
    listener: Option<thread::JoinHandle<()>>,
}

impl FakeDevice {
    /// Start a fake device on an ephemeral port. `fail_first` is the number of
    /// leading chunk POSTs to reject with 500; `corrupt` is the list of chunk
    /// indices to alter before accepting.
    fn start(fail_first: usize, corrupt: Vec<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake device");
        let port = listener.local_addr().expect("local addr").port();
        let state = Arc::new(Mutex::new(Received::default()));
        let fail_first = Arc::new(AtomicUsize::new(fail_first));
        let fail_once = Arc::new(Mutex::new(HashSet::new()));
        let corrupt = Arc::new(Mutex::new(corrupt));
        let stop = Arc::new(AtomicBool::new(false));
        let on_chunk0: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> = Arc::new(Mutex::new(None));

        let thread_state = Arc::clone(&state);
        let thread_fail = Arc::clone(&fail_first);
        let thread_fail_once = Arc::clone(&fail_once);
        let thread_corrupt = Arc::clone(&corrupt);
        let thread_on_chunk0 = Arc::clone(&on_chunk0);
        let thread_stop = Arc::clone(&stop);

        // Non-blocking accept so the loop can observe `stop` and actually
        // exit. With a blocking `incoming()`, Drop would set the flag but the
        // thread would sit in accept() forever, and joining it would hang the
        // test rather than clean up.
        listener
            .set_nonblocking(true)
            .expect("set non-blocking listener");

        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("accepted stream is blocking");
                        let s = Arc::clone(&thread_state);
                        let f = Arc::clone(&thread_fail);
                        let fo = Arc::clone(&thread_fail_once);
                        let c = Arc::clone(&thread_corrupt);
                        let h = Arc::clone(&thread_on_chunk0);
                        thread::spawn(move || {
                            let _ = handle_conn(&mut stream, &s, &f, &fo, &c, &h);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            state,
            stop,
            fail_once,
            on_chunk0,
            listener: Some(handle),
        }
    }

    /// Reject the given chunk index exactly once, so a retry must resend it.
    fn fail_chunk_once(&self, chunk_index: usize) {
        self.fail_once.lock().expect("lock").insert(chunk_index);
    }

    /// Run `f` when the device accepts chunk 0, mid-transfer.
    fn on_chunk0(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_chunk0.lock().expect("lock") = Some(Arc::new(f));
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn config(&self) -> TransferConfig {
        TransferConfig {
            base_url: self.base_url(),
            timeout_secs: 10,
            max_retries: 3,
            // Small chunks so a multi-chunk path is exercised.
            chunk_size: 64 * 1024,
            verify_checksum: true,
        }
    }

    fn received(&self) -> std::sync::MutexGuard<'_, Received> {
        self.state.lock().expect("device state lock")
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        // Signal, then join, so the port is genuinely released before the test
        // ends rather than leaking a thread into the next test.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.listener.take() {
            let _ = handle.join();
        }
    }
}

/// Read a full HTTP request (headers + body per Content-Length).
fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => head.push(byte[0]),
        }
    }
    let header_text = String::from_utf8_lossy(&head).to_string();
    let content_length = header_text
        .lines()
        .find(|l| l.to_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = vec![0u8; content_length];
    if content_length > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    Some((header_text, body))
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Extract `name="value"` from a multipart body.
fn multipart_field(body: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let needle = format!("name=\"{name}\"");
    let pos = text.find(&needle)?;
    let after = &text[pos + needle.len()..];
    let start = after.find("\r\n\r\n")? + 4;
    let end = after[start..].find("\r\n--")?;
    Some(after[start..start + end].to_string())
}

/// Extract the binary part of a multipart body (the chunk payload).
fn multipart_chunk(body: &[u8]) -> Vec<u8> {
    let marker = b"\r\n\r\n";
    let start = body
        .windows(marker.len())
        .position(|w| w == marker)
        .map(|p| p + marker.len())
        .expect("multipart body has a part");
    let end = body[start..]
        .windows(4)
        .position(|w| w == b"\r\n--")
        .map(|p| start + p)
        .unwrap_or(body.len());
    body[start..end].to_vec()
}

fn handle_conn(
    stream: &mut TcpStream,
    state: &Arc<Mutex<Received>>,
    fail_first: &Arc<AtomicUsize>,
    fail_once: &Arc<Mutex<HashSet<usize>>>,
    corrupt: &Arc<Mutex<Vec<usize>>>,
    on_chunk0: &Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    let Some((header_text, body)) = read_request(stream) else {
        return Ok(());
    };
    let path = header_text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();

    match path.as_str() {
        "/api/health" => {
            respond(
                stream,
                200,
                r#"{"status":"ok","service":"edge-ai","version":"0.1.0"}"#,
            );
        }

        "/api/models" => {
            // Report whatever the device has actually installed, so
            // `health_check` is exercised against a truthful device rather
            // than one that always claims the model is present.
            let s = state.lock().expect("lock");
            let models: Vec<serde_json::Value> = s
                .installed
                .iter()
                .map(|(name, sha)| serde_json::json!({ "name": name, "sha256": sha, "size": 0 }))
                .collect();
            let body = serde_json::json!({ "count": models.len(), "models": models });
            drop(s);
            respond(stream, 200, &body.to_string());
        }

        "/api/models/upload" => {
            let model_name = multipart_field(&body, "model_name").unwrap_or_default();
            let sha256 = multipart_field(&body, "sha256").unwrap_or_default();
            let chunk_index: usize = multipart_field(&body, "chunk_index")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let total_chunks: usize = multipart_field(&body, "total_chunks")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            if model_name.is_empty() || sha256.is_empty() || total_chunks == 0 {
                respond(stream, 400, r#"{"error":"missing required fields"}"#);
                return Ok(());
            }

            // Fault injection, matching the daemon's reset-on-chunk-0 rule.
            if chunk_index == 0 {
                let mut s = state.lock().expect("lock");
                s.declared_sha.insert(model_name.clone(), sha256);
            }
            let remaining = fail_first.load(Ordering::SeqCst);
            if remaining > 0 {
                fail_first.store(remaining - 1, Ordering::SeqCst);
                let mut s = state.lock().expect("lock");
                s.requests_per_chunk.entry(chunk_index).or_insert(0);
                *s.requests_per_chunk.get_mut(&chunk_index).expect("entry") += 1;
                s.total_posts += 1;
                drop(s);
                respond(stream, 500, r#"{"error":"injected failure"}"#);
                return Ok(());
            }

            // Fail this one chunk once, so a retry has to resend it. Used to
            // prove the client rewinds instead of resuming mid-file.
            if fail_once.lock().expect("lock").remove(&chunk_index) {
                let mut s = state.lock().expect("lock");
                *s.requests_per_chunk.entry(chunk_index).or_insert(0) += 1;
                s.total_posts += 1;
                drop(s);
                respond(stream, 500, r#"{"error":"injected one-shot failure"}"#);
                return Ok(());
            }

            let mut data = multipart_chunk(&body);
            if corrupt.lock().expect("lock").contains(&chunk_index) {
                // Flip a byte. A correct client must reject this via the
                // server-side checksum; a client that skipped verification
                // would "succeed" with a corrupt model.
                if let Some(b) = data.first_mut() {
                    *b ^= 0xFF;
                }
            }

            let mut s = state.lock().expect("lock");
            *s.requests_per_chunk.entry(chunk_index).or_insert(0) += 1;
            s.total_posts += 1;
            s.chunks.insert((model_name.clone(), chunk_index), data);
            drop(s);

            respond(
                stream,
                200,
                &format!(r#"{{"status":"chunk received","chunk_index":{chunk_index}}}"#),
            );

            // Fire the mid-transfer hook outside the state lock, since a test
            // hook is free to touch the model file itself.
            if chunk_index == 0 {
                let hook = on_chunk0.lock().expect("lock").clone();
                if let Some(hook) = hook {
                    hook();
                }
            }
        }

        "/api/models/upload/finalize" => {
            let value: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(_) => {
                    respond(stream, 400, r#"{"error":"bad json"}"#);
                    return Ok(());
                }
            };
            let model_name = value["model_name"].as_str().unwrap_or_default().to_string();
            let expected_sha = value["sha256"].as_str().unwrap_or_default().to_string();
            let total_chunks = value["total_chunks"].as_u64().unwrap_or(0) as usize;

            let mut assembled = Vec::new();
            let mut missing = None;
            {
                let mut s = state.lock().expect("lock");
                for i in 0..total_chunks {
                    match s.chunks.remove(&(model_name.clone(), i)) {
                        Some(c) => assembled.extend_from_slice(&c),
                        None => {
                            missing = Some(i);
                            break;
                        }
                    }
                }
            }
            if let Some(i) = missing {
                respond(stream, 500, &format!(r#"{{"error":"missing chunk {i}"}}"#));
                return Ok(());
            }

            // The daemon verifies the whole-file SHA-256 here and refuses a
            // mismatch. This is the only thing standing between a flaky link
            // and a silently corrupt model.
            let computed = sha256_hex_bytes(&assembled);
            if computed != expected_sha {
                let mut s = state.lock().expect("lock");
                s.corrupted.push(assembled.len());
                drop(s);
                respond(
                    stream,
                    400,
                    &format!(
                        r#"{{"error":"sha256 mismatch: expected {expected_sha} got {computed}"}}"#
                    ),
                );
                return Ok(());
            }

            let mut s = state.lock().expect("lock");
            s.installed.push((model_name.clone(), computed));
            drop(s);
            respond(
                stream,
                200,
                &format!(r#"{{"status":"model installed","model":"{model_name}"}}"#),
            );
        }

        other => {
            respond(stream, 404, &format!(r#"{{"error":"no route {other}"}}"#));
        }
    }
    Ok(())
}

/// Write a file of `size` deterministic bytes that is also a *parseable* GGUF
/// header.
///
/// `transfer_model` calls `ModelMeta::from_path` before uploading, so the test
/// model has to survive the real parser — otherwise the test would be
/// exercising the metadata path rather than the transfer. The header is
/// well-formed GGUF v3 with one metadata entry (`general.architecture`,
/// which `validate_deployment` reads) followed by pseudo-random filler.
fn write_model(dir: &std::path::Path, size: usize) -> std::path::PathBuf {
    let path = dir.join("tinyllama.q4_k_m.gguf");

    let mut data: Vec<u8> = Vec::with_capacity(size);
    data.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // magic "GGUF"
    data.extend_from_slice(&3u32.to_le_bytes()); // version
    data.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
    data.extend_from_slice(&1u64.to_le_bytes()); // metadata_kv_count = 1

    // One key/value pair: general.architecture = "aarch64" (ValueType::String
    // is 8).
    let key = b"general.architecture";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&8u32.to_le_bytes());
    let arch = b"aarch64";
    data.extend_from_slice(&(arch.len() as u64).to_le_bytes());
    data.extend_from_slice(arch);

    // Filler: reproducible pseudo-random bytes, so a corrupted chunk is
    // detectable by checksum and is not just a run of zeros.
    let mut x: u32 = 0x1234_5678;
    while data.len() < size {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.extend_from_slice(&x.to_le_bytes());
    }
    data.truncate(size);

    std::fs::write(&path, &data).expect("write model");

    // Sanity: the fixture must be one the real parser accepts, or the test is
    // measuring the wrong thing.
    let meta = aios_core::ModelMeta::from_path(&path).expect("fixture must parse as GGUF");
    assert!(meta.is_gguf(), "fixture must have a GGUF header");

    path
}

// --- Happy path ---------------------------------------------------------

#[test]
fn the_daemons_finish_health_check_route_answers() {
    // The ROADMAP criterion for Phase 7 ends with "health check ok". The
    // daemon serves GET /api/health; the mock must answer the same shape, so
    // this asserts both. The client side of the health check is exercised in
    // `deploy/tests/deploy_flow.rs`.
    let device = FakeDevice::start(0, vec![]);

    let response = reqwest::blocking::get(format!("{}/api/health", device.base_url()))
        .expect("health request");
    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().expect("health json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "edge-ai");

    let source = include_str!("../../edge-ai/src/api.rs");
    assert!(
        source.contains(r#"("GET", "/api/health")"#),
        "daemon no longer serves GET /api/health"
    );
}

#[test]
fn transfers_a_model_in_chunks_and_the_device_verifies_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 200 * 1024); // 4 chunks at 64 KiB
    let device = FakeDevice::start(0, vec![]);

    let result = transfer_model(&device.config(), "dev-01", &path, "tinyllama")
        .expect("transfer should succeed");

    assert!(result.success);
    assert_eq!(result.attempts, 1, "a healthy device needs no retry");
    assert_eq!(result.model_size, 200 * 1024);
    assert_eq!(result.model_name, "tinyllama");
    assert!(!result.error.is_some());

    // The checksum the client reported must match the file on disk.
    let on_disk = sha256_hex_bytes(&std::fs::read(&path).unwrap());
    assert_eq!(result.sha256, on_disk);

    let state = device.received();
    // Finalize succeeded, so the device stored the model under its name.
    assert_eq!(state.installed.len(), 1);
    assert_eq!(state.installed[0].0, "tinyllama");
    assert_eq!(state.installed[0].1, on_disk);
    // Four chunks, one attempt each, no duplicates.
    assert_eq!(state.total_posts, 4);
    for idx in 0..4 {
        assert_eq!(state.requests_per_chunk[&idx], 1, "chunk {idx}");
    }
}

#[test]
fn a_single_chunk_model_still_transfers() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 1024);
    let device = FakeDevice::start(0, vec![]);

    let result = transfer_model(&device.config(), "dev-01", &path, "tiny").expect("transfer");
    assert!(result.success);
    assert_eq!(result.attempts, 1);
    assert_eq!(device.received().total_posts, 1);
}

// --- Retry --------------------------------------------------------------

#[test]
fn retries_after_a_transient_device_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 128 * 1024); // 2 chunks
                                                    // Reject the first 2 chunk POSTs (i.e. the whole first attempt).
    let device = FakeDevice::start(2, vec![]);

    let result = transfer_model(&device.config(), "dev-01", &path, "tiny")
        .expect("transfer should succeed after retry");

    assert!(result.success);
    assert!(
        result.attempts > 1,
        "expected a retry, got {:?}",
        result.attempts
    );

    let state = device.received();
    assert_eq!(state.installed.len(), 1, "the model landed exactly once");
    // Attempts re-sent chunks, so more POSTs than chunks arrived.
    assert!(
        state.total_posts > 2,
        "retry should re-send chunks, saw {} posts",
        state.total_posts
    );
    // The device still holds a byte-exact model: the retried attempt was clean.
    let on_disk = sha256_hex_bytes(&std::fs::read(&path).unwrap());
    assert_eq!(state.installed[0].1, on_disk);
}

#[test]
fn gives_up_after_exhausting_retries() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 64 * 1024); // 1 chunk
                                                   // Fail far more often than the retry budget.
    let device = FakeDevice::start(99, vec![]);

    let err = transfer_model(&device.config(), "dev-01", &path, "tiny")
        .expect_err("transfer must fail when retries are exhausted");

    let message = format!("{err}");
    // max_retries is 3, i.e. one attempt plus three retries.
    assert!(
        message.contains("4 attempts"),
        "error should name the attempt count (1 + 3 retries), got: {message}"
    );
    // Nothing was installed, so no half-written model was accepted.
    assert!(device.received().installed.is_empty());
}

#[test]
fn max_retries_means_retries_after_the_first_attempt() {
    // max_retries = 0 must mean "try once, do not retry", not "never try".
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 64 * 1024);
    let device = FakeDevice::start(99, vec![]);

    let config = TransferConfig {
        max_retries: 0,
        ..device.config()
    };
    let err = transfer_model(&config, "dev-01", &path, "tiny").expect_err("one attempt must fail");
    assert!(
        format!("{err}").contains("1 attempts"),
        "max_retries=0 must be exactly one attempt, got: {err}"
    );
    assert_eq!(
        device.received().total_posts,
        1,
        "exactly one upload attempt should reach the device"
    );
}

#[test]
fn a_retry_resends_the_whole_model_not_just_the_tail() {
    // The client streams from a file handle, so a retry after a mid-transfer
    // failure has to rewind. If it did not, the second attempt would start
    // partway through the model and the device's SHA-256 check would reject it
    // — a silent data-corruption bug.
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 100 * 1024); // 3 chunks
                                                    // Fail the second chunk of the first attempt only, so the retry begins
                                                    // with chunk 0 already sent.
    let device = FakeDevice::start(0, vec![]);
    device.fail_chunk_once(1);

    let result = transfer_model(&device.config(), "dev-01", &path, "tiny")
        .expect("a retried transfer must still install the full model");
    assert!(result.attempts > 1, "expected a retry");

    let state = device.received();
    assert_eq!(state.installed.len(), 1);
    assert_eq!(
        state.installed[0].1,
        sha256_hex_bytes(&std::fs::read(&path).unwrap()),
        "the retried upload must reassemble to the exact original model"
    );
}

// --- Checksum -----------------------------------------------------------

#[test]
fn a_corrupted_transfer_is_rejected_rather_than_installed() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 64 * 1024); // 1 chunk
                                                   // The device corrupts chunk 0 before accepting it. The server-side
                                                   // SHA-256 check must catch it, so the model is never installed.
    let device = FakeDevice::start(0, vec![0]);

    let err = transfer_model(&device.config(), "dev-01", &path, "tiny")
        .expect_err("a corrupt model must not be accepted");

    let message = format!("{err}");
    assert!(
        message.contains("sha256") || message.contains("mismatch"),
        "error should mention the checksum, got: {message}"
    );
    assert!(
        device.received().installed.is_empty(),
        "a model that failed checksum verification must not be installed"
    );
}

// --- Health check -------------------------------------------------------

#[test]
fn health_check_passes_after_a_successful_transfer() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 128 * 1024);
    let device = FakeDevice::start(0, vec![]);

    let result = transfer_model(&device.config(), "dev-01", &path, "tinyllama").expect("transfer");

    let health =
        aios_deploy::health_check(&device.base_url(), &result.model_name).expect("health check");
    assert_eq!(health.status, "ok");
    assert_eq!(health.service, "edge-ai");
    assert_eq!(health.model, "tinyllama");
    assert!(health.model_present);
}

#[test]
fn health_check_fails_when_the_device_does_not_list_the_model() {
    // The device is up and healthy but has nothing installed. Reporting that
    // as a successful deployment would be wrong, so health_check must fail
    // rather than pass on liveness alone.
    let device = FakeDevice::start(0, vec![]);
    let err = aios_deploy::health_check(&device.base_url(), "never-deployed")
        .expect_err("a device without the model must fail the health check");
    let message = format!("{err}");
    assert!(
        message.contains("never-deployed"),
        "error should name the missing model, got: {message}"
    );
}

#[test]
fn health_check_fails_against_an_unreachable_device() {
    let err = aios_deploy::health_check("http://127.0.0.1:1", "tiny")
        .expect_err("an unreachable device must fail the health check");
    assert!(
        format!("{err}").contains("health") || format!("{err}").contains("connect"),
        "error should be about reaching the device, got: {err}"
    );
}

// --- Failure modes ------------------------------------------------------

#[test]
fn an_empty_model_is_rejected_before_any_network_call() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("empty.gguf");
    std::fs::write(&path, b"").unwrap();
    let device = FakeDevice::start(0, vec![]);

    let err = transfer_model(&device.config(), "dev-01", &path, "empty")
        .expect_err("an empty model must not be deployed");
    assert!(
        format!("{err}").contains("empty"),
        "error should say the file is empty, got: {err}"
    );
    assert_eq!(
        device.received().total_posts,
        0,
        "nothing should reach the device"
    );
}

#[test]
fn a_zero_chunk_size_is_rejected_rather_than_dividing_by_zero() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 4096);
    let device = FakeDevice::start(0, vec![]);

    let config = TransferConfig {
        chunk_size: 0,
        ..device.config()
    };
    let err = transfer_model(&config, "dev-01", &path, "tiny")
        .expect_err("chunk_size 0 must be an error, not a panic");
    assert!(
        format!("{err}").contains("chunk_size"),
        "error should name chunk_size, got: {err}"
    );
}

#[test]
fn a_model_changed_mid_transfer_is_reported_rather_than_ignored() {
    // verify_checksum re-reads the file after a successful upload. If the file
    // was replaced while the transfer ran, the digest we sent no longer
    // describes what is on disk, and the deploy must say so instead of
    // reporting success.
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 64 * 1024); // 1 chunk
    let device = FakeDevice::start(0, vec![]);

    // Swap the file for different content as soon as the device accepts the
    // first chunk — i.e. after the client hashed it, before it finishes.
    let swap_target = path.clone();
    device.on_chunk0(move || {
        let mut changed = std::fs::read(&swap_target).expect("read original");
        for b in changed.iter_mut().take(16) {
            *b ^= 0xFF;
        }
        std::fs::write(&swap_target, &changed).expect("write replacement");
    });

    let err = transfer_model(&device.config(), "dev-01", &path, "tiny")
        .expect_err("a model that changed mid-transfer must not report success");
    let message = format!("{err}");
    assert!(
        message.contains("changed while it was being deployed"),
        "error should explain the file changed, got: {message}"
    );
}

#[test]
fn no_verify_skips_the_post_transfer_re_read() {
    // The counter-case: with the check disabled the same race is not caught
    // locally. It still reaches the device, which is why the flag cannot make
    // a corrupt model install — this only drops the client's redundant check.
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 64 * 1024);
    let device = FakeDevice::start(0, vec![]);

    let swap_target = path.clone();
    device.on_chunk0(move || {
        let mut changed = std::fs::read(&swap_target).expect("read original");
        for b in changed.iter_mut().take(16) {
            *b ^= 0xFF;
        }
        std::fs::write(&swap_target, &changed).expect("write replacement");
    });

    let config = TransferConfig {
        verify_checksum: false,
        ..device.config()
    };
    let original_sha = aios_core::checksum::sha256_hex_bytes(&std::fs::read(&path).unwrap());
    let result = transfer_model(&config, "dev-01", &path, "tiny")
        .expect("with the local re-read disabled the transfer still completes");
    assert!(result.success);
    // The digest we sent describes the bytes that were uploaded, not the
    // replacement now on disk — which is exactly why the device's own
    // verification is the check that matters.
    assert_eq!(result.sha256, original_sha);
    assert_ne!(
        original_sha,
        aios_core::checksum::sha256_hex_bytes(&std::fs::read(&path).unwrap()),
        "precondition: the file on disk no longer matches what was sent"
    );
}

#[test]
fn a_missing_model_file_fails_before_any_network_call() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("nope.gguf");
    let device = FakeDevice::start(0, vec![]);

    let err = transfer_model(&device.config(), "dev-01", &missing, "tiny")
        .expect_err("missing file must fail");

    assert!(format!("{err}").contains("nope.gguf"));
    assert_eq!(
        device.received().total_posts,
        0,
        "must not open a connection for a file it cannot read"
    );
}

#[test]
fn a_wrong_url_fails_with_a_useful_error_not_a_hang() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_model(dir.path(), 1024);
    let device = FakeDevice::start(0, vec![]);

    // Point at a port nothing listens on.
    let mut config = device.config();
    config.base_url = "http://127.0.0.1:1".to_string();
    config.max_retries = 1;
    config.timeout_secs = 2;

    let err =
        transfer_model(&config, "dev-01", &path, "tiny").expect_err("unreachable device must fail");

    assert!(
        format!("{err}").contains("attempt"),
        "error should be about the transfer attempt, got: {err}"
    );
}

// --- Protocol fidelity --------------------------------------------------

// --- Parser hardening ---------------------------------------------------
//
// Found while writing this file: a model file with a corrupt `kv_count` made
// `gguf::parse_header` panic with "capacity overflow" rather than return an
// error, because the count went straight into `Vec::with_capacity`. Since
// `transfer_model` parses metadata on every deploy, a bad file arriving over
// the network could abort the process. These assert the parser now errors.

#[test]
fn a_corrupt_metadata_count_is_an_error_not_a_panic() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("corrupt.gguf");

    // Valid magic and version, then kv_count = u64::MAX.
    let mut data = Vec::new();
    data.extend_from_slice(&0x4655_4747u32.to_le_bytes());
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, &data).unwrap();

    let err = aios_core::ModelMeta::from_path(&path)
        .err()
        .expect("a corrupt metadata count must be an error, not a panic");
    assert!(
        format!("{err:?}").contains("metadata entries"),
        "error should name the offending field, got: {err:?}"
    );
}

#[test]
fn a_corrupt_array_count_is_an_error_not_a_panic() {
    // Same class of bug one level down: an array element count is also
    // file-controlled and also reached `Vec::with_capacity`.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("corrupt-array.gguf");

    let mut data = Vec::new();
    data.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // magic
    data.extend_from_slice(&3u32.to_le_bytes()); // version
    data.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
    data.extend_from_slice(&1u64.to_le_bytes()); // kv_count = 1
    let key = b"tokenizer.ggml.tokens";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&9u32.to_le_bytes()); // ValueType::Array
    data.extend_from_slice(&8u32.to_le_bytes()); // elem type = String
    data.extend_from_slice(&u64::MAX.to_le_bytes()); // count = u64::MAX
    std::fs::write(&path, &data).unwrap();

    let err = aios_core::ModelMeta::from_path(&path)
        .err()
        .expect("a corrupt array count must be an error, not a panic");
    assert!(
        format!("{err:?}").contains("array"),
        "error should name the array, got: {err:?}"
    );
}

#[test]
fn a_corrupt_string_length_is_an_error_not_an_oom() {
    // A huge declared string length used to reach `vec![0u8; len]`, which
    // either OOMs or panics before `read_exact` can report truncation.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("corrupt-string.gguf");

    let mut data = Vec::new();
    data.extend_from_slice(&0x4655_4747u32.to_le_bytes());
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    let key = b"tokenizer.ggml.model";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&8u32.to_le_bytes()); // ValueType::String
    data.extend_from_slice(&(u32::MAX as u64 * 8).to_le_bytes()); // huge length
    std::fs::write(&path, &data).unwrap();

    let err = aios_core::ModelMeta::from_path(&path)
        .err()
        .expect("an oversized string length must be an error");
    assert!(
        format!("{err:?}").contains("limit"),
        "error should cite the size limit, got: {err:?}"
    );
}

#[test]
fn a_string_length_that_overflows_the_address_space_is_an_error() {
    // Distinct failure from the case above: the declared length is small
    // enough to clear MAX_STRING_LEN but is u64::MAX, so narrowing to usize
    // truncates. On a 32-bit target that truncation turns into a small
    // allocation followed by a mis-parse of every following field. The parser
    // must reject it instead.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("overflowing-string.gguf");

    let mut data = Vec::new();
    data.extend_from_slice(&0x4655_4747u32.to_le_bytes());
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    let key = b"tokenizer.ggml.model";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&8u32.to_le_bytes()); // ValueType::String
    data.extend_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, &data).unwrap();

    let err = aios_core::ModelMeta::from_path(&path)
        .err()
        .expect("a length that overflows the address space must be an error");
    let message = format!("{err:?}");
    assert!(
        message.contains("limit") || message.contains("address space"),
        "error should cite the limit or the address space, got: {message}"
    );
}

#[test]
fn the_mock_speaks_the_daemons_protocol() {
    // Guards the premise of this whole file. The daemon's upload route is
    // "/api/models/upload" and its finalize route is
    // "/api/models/upload/finalize" — note the *nested* path, not
    // "/api/models/upload/finalize" built from a "/api/models" base. If
    // edge_ai::api ever changes either route, this assertion should break so
    // the mock is revisited rather than silently testing the wrong protocol.
    let source = include_str!("../../edge-ai/src/api.rs");
    assert!(
        source.contains(r#"("POST", "/api/models/upload")"#),
        "daemon no longer serves POST /api/models/upload"
    );
    assert!(
        source.contains(r#"("POST", "/api/models/upload/finalize")"#),
        "daemon no longer serves POST /api/models/upload/finalize"
    );
    // The daemon verifies the whole-file sha256 before installing.
    assert!(
        source.contains("sha256 mismatch"),
        "daemon no longer rejects a sha256 mismatch at finalize"
    );
}
