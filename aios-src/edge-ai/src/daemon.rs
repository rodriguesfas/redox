//! Daemon bootstrap shared by the `edge-ai` binary and `edge serve`.

use std::sync::Arc;
use std::time::Duration;

use crate::api::{handler, VERSION};
use crate::http;
use crate::metrics::Sampler;
use crate::runtime::{preload, Runtime};

/// Start the daemon: model preload (optional), a metrics sampler thread and
/// the HTTP server. Blocks forever serving requests.
pub fn run(
    host: &str,
    port: u16,
    preload_model: Option<&str>,
    history_every_secs: u64,
) -> std::io::Result<()> {
    let runtime = Arc::new(Runtime::new());
    runtime.log(
        "info",
        format!("edge-ai v{VERSION} starting on {host}:{port}"),
    );

    let sampler_runtime = runtime.clone();
    std::thread::spawn(move || {
        let mut sampler = Sampler::new();
        let _ = sampler.sample(); // baseline
        loop {
            std::thread::sleep(Duration::from_secs(history_every_secs.max(1)));
            let s = sampler.sample();
            sampler_runtime.push_history(&s);
        }
    });

    if let Some(model) = preload_model {
        let preload_runtime = runtime.clone();
        let model = model.to_string();
        std::thread::spawn(move || match preload(&preload_runtime, &model) {
            Ok(()) => preload_runtime.log("info", format!("preloaded model '{model}'")),
            Err(e) => {
                preload_runtime.log("error", format!("preload failed: {e}"));
                eprintln!("edge-ai: warning: {e}");
            }
        });
    }

    let boxed: Box<http::Handler> = Box::new(handler(runtime.clone()));
    let handler: &'static http::Handler = Box::leak(boxed);
    let addr = format!("{host}:{port}");
    http::serve(&addr, handler)
}
