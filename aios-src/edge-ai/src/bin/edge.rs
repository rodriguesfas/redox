//! `edge`: Edge AI OS CLI (Fase 5). Talks to the local daemon over HTTP.
//!
//!   edge status                 daemon health + metric summary
//!   edge models                 list installed models
//!   edge run <model>            [--prompt TEXT] [--max-tokens N] [--json]
//!   edge benchmark              [--model M] [--tokens N]
//!   edge logs                   [--tail N]
//!   edge monitor                [--json]  (ai-monitor)
//!   edge serve                  [--host H] [--port P] [--model M]
//!   edge install <file.gguf>   [name]
//!   edge remove <name>
//!   edge devices                                    list + probe registered devices
//!   edge deploy <model.gguf> --device <id|name>     Fase 7 transfer + health check
//!   edge update --device <id|name> --model <file>   redeploy a specific model
//!   edge help
//!
//! Daemon address: env `EDGE_BASE_URL` or `127.0.0.1:8989`.

use std::path::PathBuf;
use std::process::exit;

use edge_ai::http::Client;
use serde_json::{json, Value};

const USAGE: &str = "\
usage:
  edge status
  edge models
  edge run <model> [--prompt TEXT] [--max-tokens N] [--json]
  edge chat <model> [--prompt TEXT] [--max-tokens N] [--json]
  edge benchmark [--model M] [--tokens N]
  edge logs [--tail N]
  edge monitor [--json]
  edge serve [--host H] [--port P] [--model M]
  edge install <file.gguf> [name]
  edge remove <name>
  edge devices
  edge deploy <model.gguf> --device <id|name> [--name NAME] [--force] [--no-verify]
  edge update --device <id|name> --model <file.gguf>
  edge ci-monitor [--url URL] [--interval SECS] [--once] [--cookie-file PATH]
  edge help";

fn base_url() -> String {
    std::env::var("EDGE_BASE_URL").unwrap_or_else(|_| edge_ai::DEFAULT_CLIENT_BASE.to_string())
}

fn client() -> Client {
    match Client::from_base(&base_url()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("edge: {e}");
            exit(2);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        None => {
            eprintln!("{USAGE}");
            2
        }
        Some("status") => cmd_status(&args[1..]),
        Some("models") => cmd_models(&args[1..]),
        Some("run") => cmd_run(&args[1..]),
        Some("chat") => cmd_chat(&args[1..]),
        Some("benchmark") => cmd_benchmark(&args[1..]),
        Some("logs") => cmd_logs(&args[1..]),
        Some("monitor") => cmd_monitor(&args[1..]),
        Some("serve") => cmd_serve(&args[1..]),
        Some("install") => cmd_install(&args[1..]),
        Some("remove") => cmd_remove(&args[1..]),
        Some("devices") => cmd_devices(&args[1..]),
        Some("deploy") => cmd_deploy(&args[1..]),
        Some("update") => cmd_update(&args[1..]),
        Some("ci-monitor") => cmd_ci_monitor(&args[1..]),
        Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("unknown command: {other}\n{USAGE}");
            2
        }
    };
    exit(code);
}

fn get_json(c: &Client, path: &str) -> Result<Value, String> {
    let (status, body) = c.get(path)?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("parse response: {e}"))?;
    if status >= 400 {
        return Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or(&format!("http {status}"))
            .to_string());
    }
    Ok(v)
}

fn cmd_status(args: &[String]) -> i32 {
    if let Some(a) = args.first() {
        if a == "--help" || a == "-h" {
            println!("usage: edge status");
            return 0;
        }
    }
    let c = client();
    match get_json(&c, "/api/health") {
        Ok(h) => {
            println!(
                "edge-ai {}  status={}  uptime={}s  pid={}",
                opt(&h["version"]),
                opt(&h["status"]),
                h["uptime_s"],
                h["pid"]
            );
            println!("models dir: {}", opt(&h["models_dir"]));
            match get_json(&c, "/api/metrics") {
                Ok(m) => {
                    let s = &m["system"];
                    print!(
                        "cpu {}%  mem {} MB",
                        opt(&s["cpu_percent"]),
                        opt(&s["mem_used_mb"])
                    );
                    let inf = &m["infer"];
                    println!(
                        "   infer: req={} err={} tok={} avg={}ms last={}t/s",
                        inf["requests"],
                        inf["errors"],
                        inf["tokens"],
                        opt(&inf["avg_latency_ms"]),
                        opt(&inf["last_tps"])
                    );
                    println!("models installed: {}", m["models_count"]);
                }
                Err(e) => eprintln!("  metrics: {e}"),
            }
            0
        }
        Err(e) => {
            eprintln!("edge: daemon unreachable at {}: {e}", base_url());
            1
        }
    }
}

fn cmd_models(args: &[String]) -> i32 {
    if args.first().map(|s| s == "--help").unwrap_or(false) {
        println!("usage: edge models");
        return 0;
    }
    let c = client();
    match get_json(&c, "/api/models") {
        Ok(m) => {
            println!("{} model(s):", m["count"]);
            for mdl in m["models"].as_array().unwrap_or(&vec![]) {
                println!(
                    "  {:<24} {:<16} {}",
                    opt(&mdl["name"]),
                    opt(&mdl["architecture"]),
                    opt(&mdl["size"])
                );
            }
            if m["count"].as_u64().unwrap_or(0) == 0 {
                println!("  (none — install one with 'edge install' or 'ai install')");
            }
            0
        }
        Err(e) => {
            eprintln!("edge: {e}");
            1
        }
    }
}

fn cmd_run(args: &[String]) -> i32 {
    let Some(model) = args.first() else {
        eprintln!("usage: edge run <model> [--prompt TEXT] [--max-tokens N] [--json]");
        return 2;
    };
    if model == "--help" || model == "-h" {
        println!("usage: edge run <model> [--prompt TEXT] [--max-tokens N] [--json]");
        return 0;
    }
    let mut prompt = "Hello".to_string();
    let mut max_tokens = 64usize;
    let mut json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--prompt" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--prompt requires a value");
                    return 2;
                }
                prompt = args[i].clone();
            }
            "--max-tokens" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--max-tokens requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => max_tokens = n,
                    Err(_) => {
                        eprintln!("--max-tokens expects a number");
                        return 2;
                    }
                }
            }
            "--json" => json = true,
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }
    let c = client();
    let body = json!({ "model": model, "prompt": prompt, "max_tokens": max_tokens });
    match c.post_json("/api/infer", &body) {
        Ok((status, resp)) => {
            let v: Value = match serde_json::from_slice(&resp) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("edge: parse response: {e}");
                    return 1;
                }
            };
            if status >= 400 {
                eprintln!(
                    "edge: {}",
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("infer failed")
                );
                return 1;
            }
            if json {
                println!("{}", v);
            } else {
                let tps = v["tokens_per_second"].as_f64().unwrap_or(0.0);
                println!("{}", opt(&v["text"]));
                eprintln!(
                    "==> {}  {tps:.2} tokens/s (load {} ms, cached {})",
                    opt(&v["model"]),
                    v["load_ms"],
                    v["cached"]
                );
            }
            0
        }
        Err(e) => {
            eprintln!("edge: daemon unreachable at {}: {e}", base_url());
            1
        }
    }
}
fn cmd_chat(args: &[String]) -> i32 {
    let Some(model) = args.first() else {
        eprintln!("usage: edge chat <model> [--prompt TEXT] [--max-tokens N] [--json]");
        return 2;
    };
    if model == "--help" || model == "-h" {
        println!("usage: edge chat <model> [--prompt TEXT] [--max-tokens N] [--json]");
        return 0;
    }
    let mut prompt = "Hello".to_string();
    let mut max_tokens = 256usize;
    let mut json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--prompt" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--prompt requires a value");
                    return 2;
                }
                prompt = args[i].clone();
            }
            "--max-tokens" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--max-tokens requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => max_tokens = n,
                    Err(_) => {
                        eprintln!("--max-tokens expects a number");
                        return 2;
                    }
                }
            }
            "--json" => json = true,
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }
    let c = client();
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens
    });
    match c.post_json("/v1/chat/completions", &body) {
        Ok((status, resp)) => {
            let v: Value = match serde_json::from_slice(&resp) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("edge: parse response: {e}");
                    return 1;
                }
            };
            if status >= 400 {
                eprintln!(
                    "edge: {}",
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("chat failed")
                );
                return 1;
            }
            if json {
                println!("{}", v);
            } else {
                let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
                println!("{content}");
                eprintln!(
                    "==> model={} tokens={}",
                    opt(&v["model"]),
                    v["usage"]["total_tokens"]
                );
            }
            0
        }
        Err(e) => {
            eprintln!("edge: daemon unreachable at {}: {e}", base_url());
            1
        }
    }
}

fn cmd_benchmark(args: &[String]) -> i32 {
    let mut model = String::new();
    let mut tokens = 0usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--model requires a value");
                    return 2;
                }
                model = args[i].clone();
            }
            "--tokens" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--tokens requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => tokens = n,
                    Err(_) => {
                        eprintln!("--tokens expects a number");
                        return 2;
                    }
                }
            }
            "--help" | "-h" => {
                println!("usage: edge benchmark [--model M] [--tokens N]");
                return 0;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }
    let mut path = "/api/benchmark?".to_string();
    let mut first = true;
    if !model.is_empty() {
        path.push_str(&format!("model={model}"));
        first = false;
    }
    if tokens > 0 {
        if !first {
            path.push('&');
        }
        path.push_str(&format!("tokens={tokens}"));
    }
    let c = client();
    match get_json(&c, &path) {
        Ok(b) => {
            println!("model:       {}", opt(&b["model"]));
            println!("size:        {}", opt(&b["size"]));
            println!("gguf parse:  {} ms", b["gguf_parse_ms"]);
            println!(
                "sha256:      {} ms  {}",
                b["sha256_ms"],
                b["sha256"].as_str().map(|s| &s[..16]).unwrap_or("")
            );
            if let Some(tps) = b["tokens_per_second"].as_f64() {
                println!(
                    "infer:       {tps:.2} tokens/s ({} tok, load {} ms, cached {})",
                    b["tokens"], b["load_ms"], b["cached"]
                );
            } else if let Some(e) = b["tokens_error"].as_str() {
                println!("infer:       skipped ({e})");
            }
            0
        }
        Err(e) => {
            eprintln!("edge: {e}");
            1
        }
    }
}

fn cmd_logs(args: &[String]) -> i32 {
    let mut tail = 0usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tail" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--tail requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => tail = n,
                    Err(_) => {
                        eprintln!("--tail expects a number");
                        return 2;
                    }
                }
            }
            "--help" | "-h" => {
                println!("usage: edge logs [--tail N]");
                return 0;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }
    let mut path = "/api/logs".to_string();
    if tail > 0 {
        path.push_str(&format!("?tail={tail}"));
    }
    let c = client();
    match get_json(&c, &path) {
        Ok(l) => {
            if l["count"].as_u64().unwrap_or(0) == 0 {
                println!("(no log entries yet)");
                return 0;
            }
            for e in l["logs"].as_array().unwrap_or(&vec![]) {
                println!("[{}] {}", opt(&e["level"]), opt(&e["msg"]));
            }
            0
        }
        Err(e) => {
            eprintln!("edge: {e}");
            1
        }
    }
}

fn cmd_monitor(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: edge monitor [--json]");
        return 0;
    }
    let c = client();
    match get_json(&c, "/api/metrics") {
        Ok(m) => {
            if json {
                println!("{}", m);
                return 0;
            }
            let s = &m["system"];
            let inf = &m["infer"];
            println!("AIOS ai-monitor  ts={}", m["ts"]);
            println!(
                "  system:  cpu {}%   mem {} MB / {} MB",
                opt(&s["cpu_percent"]),
                opt(&s["mem_used_mb"]),
                opt(&s["mem_total_mb"])
            );
            println!(
                "           net rx {} kb/s  tx {} kb/s   cores {}",
                opt(&s["net_rx_kbps"]),
                opt(&s["net_tx_kbps"]),
                s["parallelism"]
            );
            println!(
                "  infer:   requests {}  errors {}  tokens {}  avg {} ms  last {} t/s",
                inf["requests"],
                inf["errors"],
                inf["tokens"],
                opt(&inf["avg_latency_ms"]),
                opt(&inf["last_tps"])
            );
            println!("  models:  {}", m["models_count"]);
            let h = m["history"].as_array().map(|a| a.len()).unwrap_or(0);
            println!("  history: {h} points");
            0
        }
        Err(e) => {
            eprintln!("edge: {e}");
            1
        }
    }
}

fn cmd_serve(args: &[String]) -> i32 {
    let mut host = edge_ai::DEFAULT_BIND.to_string();
    let mut port = edge_ai::DEFAULT_PORT;
    let mut model: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--host requires a value");
                    return 2;
                }
                host = args[i].clone();
            }
            "--port" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--port requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(p) => port = p,
                    Err(_) => {
                        eprintln!("--port expects a number");
                        return 2;
                    }
                }
            }
            "--model" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--model requires a value");
                    return 2;
                }
                model = Some(args[i].clone());
            }
            "--help" | "-h" => {
                println!("usage: edge serve [--host H] [--port P] [--model M]");
                return 0;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }
    if let Err(e) = edge_ai::daemon::run(&host, port, model.as_deref(), 5) {
        eprintln!("edge: {e}");
        return 1;
    }
    0
}

fn cmd_install(args: &[String]) -> i32 {
    let Some(src) = args.first() else {
        eprintln!("usage: edge install <file.gguf> [name]");
        return 2;
    };
    if src == "--help" || src == "-h" {
        println!("usage: edge install <file.gguf> [name]");
        return 0;
    }
    let name = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| aios_core::default_name_for(std::path::Path::new(src)));
    let models_dir = aios_core::default_models_dir();
    let dir = std::path::Path::new(&models_dir);
    match aios_core::install_model(std::path::Path::new(src), dir, &name) {
        Ok(entry) => {
            let mut reg = match aios_core::Registry::load(aios_core::default_registry_file()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("edge: registry: {e}");
                    return 1;
                }
            };
            reg.add(entry.clone());
            if let Err(e) = reg.save() {
                eprintln!("edge: registry save: {e}");
                return 1;
            }
            println!("installed '{}' ({} bytes)", entry.name, entry.size_bytes);
            0
        }
        Err(e) => {
            eprintln!("edge: {e}");
            1
        }
    }
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: edge remove <name>");
        return 2;
    };
    if name == "--help" || name == "-h" {
        println!("usage: edge remove <name>");
        return 0;
    }
    let models_dir = aios_core::default_models_dir();
    let cache = std::path::Path::new(&models_dir);
    let mut reg = match aios_core::Registry::load(aios_core::default_registry_file()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("edge: registry: {e}");
            return 1;
        }
    };
    let Some(entry) = reg.find(name).cloned() else {
        eprintln!("edge: model '{name}' not in the registry");
        return 1;
    };
    if let Err(e) = aios_core::remove_model(&entry, cache) {
        eprintln!("edge: {e}");
        return 1;
    }
    reg.remove(name);
    if let Err(e) = reg.save() {
        eprintln!("edge: registry save: {e}");
        return 1;
    }
    println!("removed '{name}'");
    0
}

/// `edge devices`: list the deployment registry, then probe each device's
/// daemon so the listing shows reachability and health rather than only what
/// was recorded at registration time.
fn cmd_devices(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: edge devices");
        println!();
        println!("Lists every device in the deployment registry and probes each one.");
        println!("  reachable  the daemon answered /api/health");
        println!("  models     how many models the device reports installed");
        return 0;
    }
    if !args.is_empty() {
        eprintln!("edge devices takes no arguments");
        return 2;
    }

    let reg = match aios_deploy::DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("edge: could not read the device registry: {e}");
            return 1;
        }
    };

    let devices = reg.list();
    if devices.is_empty() {
        println!("No devices registered.");
        println!();
        println!("Register one with:");
        println!("  aios-deploy add <name> <host:port> [--arch ARCH] [--ram MB] [--storage MB]");
        return 0;
    }

    // No trailing placeholder: the header has one slot per column and nothing
    // for the free-text DETAIL column.
    println!(
        "{:<20} {:<22} {:<10} {:>6} {:>8}  DETAIL",
        "NAME", "ADDRESS", "REACHABLE", "MODELS", "STATUS"
    );
    for d in &devices {
        let (reachable, models, detail) = probe_device(&d.address);
        println!(
            "{:<20} {:<22} {:<10} {:>6} {:>8}  {}",
            d.name,
            d.address,
            if reachable { "yes" } else { "no" },
            models,
            d.status_str(),
            detail
        );
    }
    0
}

/// Probe one device. Returns (reachable, installed model count, detail).
fn probe_device(address: &str) -> (bool, String, String) {
    let client = match edge_ai::http::Client::from_base(address) {
        Ok(c) => c,
        Err(e) => return (false, "-".to_string(), format!("bad address: {e}")),
    };
    let health = match get_json(&client, "/api/health") {
        Ok(v) => v,
        Err(e) => return (false, "-".to_string(), e),
    };
    let status = health
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown")
        .to_string();
    let service = health
        .get("service")
        .and_then(|s| s.as_str())
        .unwrap_or("?")
        .to_string();

    // One request, two answers: the count and the names come from the same
    // response instead of asking the device twice and risking a listing that
    // disagrees with itself.
    let models_json = match get_json(&client, "/api/models") {
        Ok(v) => v,
        Err(e) => {
            return (
                true,
                "?".to_string(),
                format!("{service}/{status}; models: {e}"),
            )
        }
    };
    let names: Vec<String> = models_json
        .get("models")
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let count = models_json
        .get("count")
        .and_then(|c| c.as_u64())
        .map(|c| c.to_string())
        .unwrap_or_else(|| names.len().to_string());

    let detail = if names.is_empty() {
        format!("{service}/{status}")
    } else {
        format!("{service}/{status} {}", names.join(", "))
    };
    (status == "ok", count, detail)
}

/// `edge deploy <model.gguf> --device <id|name>`: run the Fase 7 deploy pipeline
/// against a registered device, then record the outcome in the registry.
fn cmd_deploy(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: edge deploy <model-path> --device <id|name> [--name NAME] [--force] [--no-verify]");
        println!();
        println!("Transfers a model to a registered device, waits for the daemon to report");
        println!("it healthy with the model loaded, and records the result in the registry.");
        return 0;
    }
    if args.is_empty() {
        eprintln!("usage: edge deploy <model-path> --device <id|name> [--name NAME] [--force] [--no-verify]");
        return 2;
    }

    let mut model_path: Option<String> = None;
    let mut device: Option<String> = None;
    let mut name: Option<String> = None;
    let mut force = false;
    let mut verify = true;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--device" | "-d" => match it.next() {
                Some(v) => device = Some(v.clone()),
                None => {
                    eprintln!("--device needs a value");
                    return 2;
                }
            },
            "--name" => match it.next() {
                Some(v) => name = Some(v.clone()),
                None => {
                    eprintln!("--name needs a value");
                    return 2;
                }
            },
            "--force" => force = true,
            "--no-verify" => verify = false,
            other if other.starts_with('-') => {
                eprintln!("unknown option: {other}");
                return 2;
            }
            other => {
                if model_path.is_some() {
                    eprintln!("unexpected extra argument: {other}");
                    return 2;
                }
                model_path = Some(other.to_string());
            }
        }
    }

    let model_path = match model_path {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: edge deploy <model-path> --device <id|name>");
            return 2;
        }
    };
    let needle = match device {
        Some(d) => d,
        None => {
            eprintln!("--device is required; run 'edge devices' to see registered devices");
            return 2;
        }
    };

    let reg = match aios_deploy::DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("edge: could not read the device registry: {e}");
            return 1;
        }
    };
    let found = reg
        .get(&needle)
        .or_else(|| reg.find_by_name(&needle))
        .or_else(|| {
            let matches: Vec<_> = reg
                .list()
                .into_iter()
                .filter(|d| d.id.starts_with(&needle))
                .collect();
            match matches.as_slice() {
                [only] => Some(*only),
                [] => None,
                many => {
                    eprintln!(
                        "device id prefix '{needle}' is ambiguous: {}",
                        many.iter()
                            .map(|d| format!("{} ({})", d.name, &d.id[..8]))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    None
                }
            }
        });
    let device = match found {
        Some(d) => d,
        None => {
            eprintln!("device '{needle}' not found; run 'edge devices' to list them");
            return 1;
        }
    };

    // Compatibility gate, same rule as aios-deploy: refuse an obviously
    // impossible deploy unless the operator forces it.
    match aios_deploy::validate_deployment(device, &model_path) {
        Ok(check) => {
            if !check.compatible && !force {
                eprintln!("deployment validation failed (use --force to override):");
                for e in &check.errors {
                    eprintln!("  ERROR: {e}");
                }
                return 1;
            }
            for w in &check.warnings {
                println!("  WARNING: {w}");
            }
        }
        Err(e) => {
            eprintln!("validation error: {e}");
            return 1;
        }
    }

    let name = name.unwrap_or_else(|| aios_core::default_name_for(&model_path));
    let config = aios_deploy::TransferConfig {
        base_url: format!("http://{}", device.address),
        verify_checksum: verify,
        ..Default::default()
    };

    println!(
        "deploying {name} to {} ({})...",
        device.name, device.address
    );
    let result = match aios_deploy::transfer_model(&config, &device.id, &model_path, &name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("transfer failed: {e}");
            return 1;
        }
    };
    println!(
        "  transferred {} bytes in {} attempt(s), sha256 {}",
        result.bytes_transferred, result.attempts, result.sha256
    );

    match aios_deploy::health_check(&config.base_url, &result.model_name) {
        Ok(h) => {
            println!(
                "  health ok: {}/{} with {} loaded",
                h.service, h.status, h.model
            );
        }
        Err(e) => {
            eprintln!("  health check failed: {e}");
            return 1;
        }
    }

    if let Ok(mut reg) = aios_deploy::DeviceRegistry::load() {
        let mut models = reg
            .get(&device.id)
            .map(|d| d.models.clone())
            .unwrap_or_default();
        if !models.contains(&result.model_name) {
            models.push(result.model_name.clone());
        }
        reg.update_status(&device.id, aios_deploy::DeviceStatus::Online);
        reg.update_models(&device.id, models);
        reg.touch(&device.id);
        if let Err(e) = reg.save() {
            eprintln!("warning: could not update the device registry: {e}");
        }
    }
    println!("done.");
    0
}

/// `edge update`: redeploy the newest model file to a device. Refuses to guess
/// which file that is, because picking the wrong one silently is worse than
/// asking.
fn cmd_update(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: edge update --device <id|name> --model <model-path>");
        println!();
        println!("Redeploys a specific model to a registered device.");
        println!("'edge update' will not guess which local file you mean; pass --model.");
        return 0;
    }

    let mut device: Option<String> = None;
    let mut model: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--device" | "-d" => device = it.next().cloned(),
            "--model" | "-m" => model = it.next().cloned(),
            other => {
                eprintln!("unknown option: {other}");
                return 2;
            }
        }
    }
    let (device, model) = match (device, model) {
        (Some(d), Some(m)) => (d, m),
        _ => {
            eprintln!("usage: edge update --device <id|name> --model <model-path>");
            eprintln!("edge update will not guess which local model file you mean; pass --model.");
            return 2;
        }
    };

    let mut deploy_args = vec![model, "--device".to_string(), device];
    deploy_args.push("--force".to_string());
    cmd_deploy(&deploy_args)
}

/// `edge ci-monitor`: poll CI/deploy status and send desktop notifications
fn cmd_ci_monitor(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: edge ci-monitor [--url URL] [--path PATH] [--interval SECS] [--once] [--cookie-file PATH]");
        println!();
        println!("Monitors CI/deploy status at the given URL and sends desktop notifications");
        println!("on status changes (success, failure, start).");
        println!();
        println!("Options:");
        println!("  --url URL           CI endpoint base URL (default: http://10.8.0.9:15201)");
        println!("  --path PATH         API endpoint path (default: /ci)");
        println!("  --interval SECS     Poll interval in seconds (default: 30)");
        println!("  --once              Check once and exit (don't run continuous loop)");
        println!("  --cookie-file PATH  Path to Netscape-format cookie file for authentication");
        println!("  --notify-success    Send notification on success (default: true)");
        println!("  --no-notify-success Disable success notifications");
        println!("  --notify-failure    Send notification on failure (default: true)");
        println!("  --no-notify-failure Disable failure notifications");
        println!("  --notify-start      Send notification when build starts (default: false)");
        return 0;
    }

    let mut url = "http://10.8.0.9:15201".to_string();
    let mut endpoint_path = "/ci".to_string();
    let mut interval = 30u64;
    let mut once = false;
    let mut cookie_file: Option<String> = None;
    let mut notify_success = true;
    let mut notify_failure = true;
    let mut notify_start = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--url requires a value");
                    return 2;
                }
                url = args[i].clone();
            }
            "--path" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--path requires a value");
                    return 2;
                }
                endpoint_path = args[i].clone();
            }
            "--interval" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--interval requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => interval = n,
                    Err(_) => {
                        eprintln!("--interval expects a number");
                        return 2;
                    }
                }
            }
            "--once" => once = true,
            "--cookie-file" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--cookie-file requires a path");
                    return 2;
                }
                cookie_file = Some(args[i].clone());
            }
            "--notify-success" => notify_success = true,
            "--no-notify-success" => notify_success = false,
            "--notify-failure" => notify_failure = true,
            "--no-notify-failure" => notify_failure = false,
            "--notify-start" => notify_start = true,
            other => {
                eprintln!("unknown option: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let config = edge_ai::ci_monitor::CiMonitorConfig {
        base_url: url,
        endpoint_path,
        poll_interval_secs: interval,
        notify_on_success: notify_success,
        notify_on_failure: notify_failure,
        notify_on_start: notify_start,
        cookie_file,
    };

    let mut monitor = match edge_ai::ci_monitor::CiMonitor::new(config) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("edge: failed to create CI monitor: {e}");
            return 1;
        }
    };

    if once {
        match monitor.poll_once() {
            Ok(changed) => {
                if changed {
                    println!("Status changed, notification sent");
                } else {
                    println!("No status change");
                }
                0
            }
            Err(e) => {
                eprintln!("edge: {e}");
                1
            }
        }
    } else if let Err(e) = monitor.run() {
        eprintln!("edge: {e}");
        1
    } else {
        0
    }
}

fn opt(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "–".to_string(),
        Value::Bool(b) => b.to_string(),
        _ => "?".to_string(),
    }
}
