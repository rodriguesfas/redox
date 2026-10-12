//! AIOS `ai` CLI.
//!
//! Subcommands:
//!   list       - list installed models (cache dir + registry)
//!   install    - copy a GGUF into the cache and register it
//!   remove     - unregister and delete a model from the cache
//!   info       - details about an installed model (registry + file)
//!   inspect    - print GGUF header of a model file (magic, version, tensors, metadata KV)
//!   verify     - compute and print the SHA-256 of a model file
//!   benchmark  - micro-benchmark: GGUF parse, SHA-256, and Candle CPU backend
//!   run        - run inference with the Candle CPU backend
//!   task       - run a named AITask (summarize/classify/...) via routing config
//!   serve      - (Fase 4) expose a local HTTP endpoint
//!   stop       - (Fase 4) stop a running model/server
//!   doctor     - environment checks (toolchain, cache dir, registry)

mod tasks;

use aios_core::default_models_dir;
use aios_core::gguf::ValueType;
use aios_core::{
    self, cache_path_for, find_models, install_model, list_installed, pretty_bytes, remove_model,
    Registry, RegistryEntry,
};
use aios_inference::ComputeBackend;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

const USAGE: &str = "\
AIOS ai - model cache and inspection CLI

Usage:
  ai list                  [--dir DIR]
  ai install <file.gguf>   [name] [--dir DIR]
  ai remove <name>         [--dir DIR]
  ai info <name>           [--dir DIR]
  ai inspect <file.gguf>
  ai verify <file.gguf>
  ai benchmark <file.gguf> [--iter N]
  ai run <model>
  ai task <name> [--list | --model M | --text T | --file F] [--max-tokens N] [--json]
  ai serve | stop         (Fase 4)
  ai doctor
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let code = match args[0].as_str() {
        "list" => cmd_list(&args[1..]),
        "install" => cmd_install(&args[1..]),
        "remove" => cmd_remove(&args[1..]),
        "info" => cmd_info(&args[1..]),
        "inspect" => cmd_inspect(&args[1..]),
        "verify" => cmd_verify(&args[1..]),
        "benchmark" => cmd_benchmark(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "task" => cmd_task(&args[1..]),
        "serve" | "stop" => cmd_fase4(&args[0]),
        "doctor" => cmd_doctor(),
        "help" | "-h" | "--help" => {
            eprintln!("{USAGE}");
            0
        }
        other => {
            eprintln!("unknown command: {other}\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// Parse a `--dir DIR` argument from a positional arg stream.
fn take_dir(args: &[String], i: &mut usize, default: &str) -> Result<String, String> {
    if args.get(*i).map(|a| a.as_str()) == Some("--dir") {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| "--dir requires a path".to_string())
    } else {
        Ok(default.to_string())
    }
}

fn cmd_install(args: &[String]) -> i32 {
    let Some(src) = args.first() else {
        eprintln!("usage: ai install <file.gguf> [name] [--dir DIR]");
        return 2;
    };
    let mut name: Option<String> = None;
    let mut dir = default_models_dir();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--dir requires a path");
                    return 2;
                }
                dir = args[i].clone();
            }
            other if name.is_none() => {
                name = Some(other.to_string());
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let name = name.unwrap_or_else(|| aios_core::default_name_for(Path::new(src)));

    println!("Installing {src} as '{name}' in {dir}...");
    match install_model(Path::new(src), Path::new(&dir), &name) {
        Ok(entry) => {
            let mut reg = match Registry::load(aios_core::default_registry_file()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: registry: {e}");
                    return 1;
                }
            };
            reg.add(entry.clone());
            if let Err(e) = reg.save() {
                eprintln!("error: saving registry: {e}");
                return 1;
            }
            println!("  name:   {name}");
            println!("  path:   {}", entry.path);
            println!("  sha256: {}", entry.sha256);
            println!(
                "  size:   {} ({} bytes)",
                pretty_bytes(entry.size_bytes),
                entry.size_bytes
            );
            println!("Installed.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: ai remove <name> [--dir DIR]");
        return 2;
    };
    let dir = registry_dir(args);

    let mut reg = match Registry::load(aios_core::default_registry_file()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: registry: {e}");
            return 1;
        }
    };

    if let Some(entry) = reg.find(name) {
        let entry = entry.clone();
        if let Err(e) = remove_model(&entry, Path::new(&dir)) {
            eprintln!("error: {e}");
            return 1;
        }
        reg.remove(name);
        if let Err(e) = reg.save() {
            eprintln!("error: saving registry: {e}");
            return 1;
        }
        println!("Removed {name} ({}).", entry.path);
        return 0;
    }

    // Not registered: try a plain cache file with this name.
    let guess = cache_path_for(Path::new(&dir), name);
    if guess.exists() {
        if let Err(e) = remove_model(
            &RegistryEntry {
                name: name.to_string(),
                path: guess.display().to_string(),
                sha256: String::new(),
                size_bytes: 0,
            },
            Path::new(&dir),
        ) {
            eprintln!("error: {e}");
            return 1;
        }
        println!("Removed unregistered model {name}.");
        return 0;
    }

    eprintln!("error: model '{name}' not found in registry or {dir}");
    1
}

fn cmd_info(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: ai info <name> [--dir DIR]");
        return 2;
    };
    let dir = registry_dir(args);

    let reg = match Registry::load(aios_core::default_registry_file()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: registry: {e}");
            return 1;
        }
    };

    if let Some(e) = reg.find(name) {
        let exists = Path::new(&e.path).exists();
        println!("Name:        {name}");
        println!("Registered:  yes");
        println!("Path:        {}", e.path);
        println!(
            "Size:        {} ({} bytes)",
            pretty_bytes(e.size_bytes),
            e.size_bytes
        );
        println!("SHA-256:     {}", e.sha256);
        println!("On disk:     {}", if exists { "yes" } else { "no" });
        return if exists { 0 } else { 1 };
    }

    let guess = cache_path_for(Path::new(&dir), name);
    if guess.exists() {
        let size = std::fs::metadata(&guess).map(|m| m.len()).unwrap_or(0);
        println!("Name:        {name}");
        println!("Registered:  no");
        println!("Path:        {}", guess.display());
        println!("Size:        {} ({} bytes)", pretty_bytes(size), size);
        return 0;
    }

    eprintln!("error: model '{name}' not found in registry or {dir}");
    1
}

fn registry_dir(args: &[String]) -> String {
    let mut d = default_models_dir();
    let mut i = 0;
    match take_dir(args, &mut i, &d) {
        Ok(nd) => d = nd,
        Err(e) => eprintln!("{e}"),
    }
    d
}

fn cmd_list(args: &[String]) -> i32 {
    let mut dir = default_models_dir();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--dir requires a path");
                    return 2;
                }
                dir = args[i].clone();
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    println!("Models in {dir}:");
    match list_installed(Path::new(&dir)) {
        Ok(models) => {
            if models.is_empty() {
                println!("  (none)");
            }
            for m in &models {
                let kind = if m.is_gguf() {
                    "gguf"
                } else {
                    "unknown/unreadable"
                };
                println!("  {:<28} {:>10}  {}", m.name, m.size_pretty(), kind);
            }

            let reg_file = aios_core::default_registry_file();
            match Registry::load(&reg_file) {
                Ok(reg) => {
                    if !reg.entries().is_empty() {
                        println!("\nRegistry ({reg_file}):");
                        for e in reg.entries() {
                            println!(
                                "  {:<28} {:<20} {}",
                                e.name,
                                e.sha256[..10.min(e.sha256.len())].to_string(),
                                pretty_bytes(e.size_bytes)
                            );
                        }
                    }
                }
                Err(e) => eprintln!("warning: registry: {e}"),
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_inspect(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: ai inspect <file.gguf>");
        return 2;
    };
    let model = match aios_core::ModelMeta::from_path(Path::new(path)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!("Path:        {}", model.path);
    println!("Name:        {}", model.name);
    println!("Size:        {}", model.size_pretty());

    match &model.header {
        None => {
            println!("Format:      not GGUF");
            1
        }
        Some(h) => {
            println!("Format:      GGUF");
            println!("Version:     {}", h.version);
            println!("Tensors:     {}", h.tensor_count);
            println!("Metadata:    {} entries", h.metadata.len());
            println!("\nMetadata:");
            for m in &h.metadata {
                println!(
                    "  {:<32} {:<12} {}",
                    m.key,
                    type_name(m.value_type),
                    m.value.brief()
                );
            }
            0
        }
    }
}

fn cmd_verify(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: ai verify <file.gguf>");
        return 2;
    };
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!(
        "{}",
        aios_core::checksum::sha256_hex(file).unwrap_or_else(|e| format!("error: {e}"))
    );
    0
}

/// Fase 3 micro-benchmark of the metadata/checksum path (the only CPU work
/// available before the inference backend). Reports parse rate and SHA-256
/// throughput as a baseline; inference benchmarks arrive in Fase 4.
fn cmd_benchmark(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: ai benchmark <file.gguf> [--iter N]");
        return 2;
    };
    let mut iter = 200usize;
    let mut gen_tokens = 32usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--iter" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--iter requires a number");
                    return 2;
                }
                iter = args[i].parse().unwrap_or(200);
            }
            "--gen-tokens" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--gen-tokens requires a number");
                    return 2;
                }
                gen_tokens = args[i].parse().unwrap_or(32);
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);

    // Parse benchmark: header only (the full tensor data does not need
    // parsing). We re-open the file each iteration so metadata regions larger
    // than any buffer are handled (real vocabularies exceed 1 MiB).
    let n_parse = iter;
    let t0 = Instant::now();
    let mut parsed_ok = 0usize;
    for _ in 0..n_parse {
        let ok = match std::fs::File::open(path) {
            Ok(mut f) => aios_core::gguf::parse_header(&mut f).is_ok(),
            Err(_) => false,
        };
        if ok {
            parsed_ok += 1;
        }
    }
    let parse_secs = t0.elapsed().as_secs_f64();

    // SHA-256 throughput on a fixed sample of the file.
    let head = {
        let mut file = file;
        const HEAD: usize = 1 << 20;
        let mut buf = vec![0u8; HEAD];
        match file.read(&mut buf) {
            Ok(n) => buf[..n].to_vec(),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };
    let n_hash = iter;
    let t1 = Instant::now();
    for _ in 0..n_hash {
        aios_core::checksum::sha256_hex(head.as_slice()).ok();
    }
    let hash_secs = t1.elapsed().as_secs_f64();
    let total_bytes = (head.len() as u64) * (n_hash as u64);
    let mib_per_s = (total_bytes as f64) / (1 << 20) as f64 / hash_secs;

    println!("AIOS ai benchmark (metadata/checksum/Candle backend)");
    println!("  file:               {path}");
    println!(
        "  size:               {} ({} bytes)",
        pretty_bytes(size),
        size
    );
    println!("  iter:               {iter}");
    println!(
        "  GGUF parse:         {parsed_ok}/{n_parse} ok, {:.1} parse/s",
        n_parse as f64 / parse_secs.max(1e-9)
    );
    println!(
        "  SHA-256:            {:.2} MiB/s (sample {} bytes)",
        mib_per_s,
        head.len()
    );
    infer_benchmark(path, gen_tokens);
    0
}

/// Second half of `ai benchmark`: load the model through the Candle backend
/// and measure real generation throughput. Non-fatal (reports why it skipped).
fn infer_benchmark(path: &str, max_tokens: usize) {
    match aios_inference::CandleBackend::new() {
        Ok(mut backend) => match backend.load_model(path) {
            Ok(()) => {
                let name = aios_inference::model_name(&backend).to_string();
                let load_ms = aios_inference::load_time(&backend).as_millis();
                let prompt = "Hello";
                let out = match backend.generate(prompt, max_tokens) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("  Candle:             load ok but generation failed: {e}");
                        return;
                    }
                };
                let tps = backend.tokens_per_second();
                println!(
                    "  Candle backend:     {} on {} (load {load_ms} ms)",
                    backend.name(),
                    backend.device()
                );
                println!("  Candle model:       {name}");
                println!("  Candle generate:    prompt={prompt:?} -> {out:?}");
                println!("  Candle throughput:  {tps:.2} tokens/s");
            }
            Err(e) => println!("  Candle backend:     skipped ({e})"),
        },
        Err(e) => println!("  Candle backend:     unavailable ({e})"),
    }
}

/// `ai run <model>` — run inference through the Candle CPU backend.
fn cmd_run(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: ai run <model|path.gguf> [--prompt TEXT] [--max-tokens N]");
        return 2;
    };
    let mut prompt = "Hello".to_string();
    let mut max_tokens = 64usize;
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
                max_tokens = match args[i].parse() {
                    Ok(n) => n,
                    Err(_) => {
                        eprintln!("--max-tokens expects a number");
                        return 2;
                    }
                };
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let path = match resolve_model_path(name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let mut backend = match aios_inference::CandleBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: backend init: {e}");
            return 1;
        }
    };
    if let Err(e) = backend.load_model(&path) {
        eprintln!("error: load {path}: {e}");
        return 1;
    }
    eprintln!(
        "==> {name}  [{}, load {} ms]",
        backend.device(),
        aios_inference::load_time(&backend).as_millis()
    );
    match backend.generate(&prompt, max_tokens) {
        Ok(text) => {
            println!("{text}");
            eprintln!("==> {:.2} tokens/s", backend.tokens_per_second());
            0
        }
        Err(e) => {
            eprintln!("error: generate: {e}");
            1
        }
    }
}

fn cmd_fase4(name: &str) -> i32 {
    eprintln!(
        "error: 'ai {name}' requires the Fase 4 serving stack (HTTP/daemon), which is \
         not implemented yet (see docs/ROADMAP.md). Available now: list, install, \
         remove, info, inspect, verify, benchmark, run, task, doctor."
    );
    1
}

/// Resolve a model reference to a local path: explicit file, registry entry,
/// or `<cache>/<name>.gguf` fallback.
fn resolve_model_path(name: &str) -> Result<String, String> {
    if std::path::Path::new(name).is_file() {
        return Ok(name.to_string());
    }
    let reg =
        Registry::load(aios_core::default_registry_file()).map_err(|e| format!("registry: {e}"))?;
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

/// `ai task <name>` — run a named AITask through the Candle CPU backend.
/// Input precedence: `--text` > `--file` > stdin. `--list` prints all tasks.
fn cmd_task(args: &[String]) -> i32 {
    if args.first().map(|s| s.as_str()) == Some("--list")
        || args.first().map(|s| s.as_str()) == Some("list")
    {
        let router = match tasks::TaskRouter::load(&tasks::TaskRouter::default_tasks_file()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        println!("Available tasks ({})", router.all().len());
        for t in router.all() {
            let model = t.model.as_deref().unwrap_or("<unset>");
            println!(
                "  {:<12} {:<40} model={model} max_tokens={}",
                t.name, t.description, t.max_tokens
            );
        }
        return 0;
    }

    let Some(name) = args.first() else {
        eprintln!("usage: ai task <name> [--list] [--model M] [--text T | --file F] [--max-tokens N] [--json]");
        return 2;
    };
    let mut model_arg: Option<String> = None;
    let mut text: Option<String> = None;
    let mut file: Option<String> = None;
    let mut max_tokens: Option<usize> = None;
    let mut json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--model requires a value");
                    return 2;
                }
                model_arg = Some(args[i].clone());
            }
            "--text" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--text requires a value");
                    return 2;
                }
                text = Some(args[i].clone());
            }
            "--file" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--file requires a path");
                    return 2;
                }
                file = Some(args[i].clone());
            }
            "--max-tokens" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--max-tokens requires a number");
                    return 2;
                }
                match args[i].parse() {
                    Ok(n) => max_tokens = Some(n),
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

    let router = match tasks::TaskRouter::load(&tasks::TaskRouter::default_tasks_file()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let spec = match router.get(name) {
        Some(s) => s,
        None => {
            eprintln!("unknown task '{name}'");
            eprintln!("run 'ai task --list' to see the available tasks.");
            return 1;
        }
    };

    // Input: --text > --file > stdin.
    let input = if let Some(t) = text {
        t
    } else if let Some(f) = file {
        match std::fs::read_to_string(&f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: read {f}: {e}");
                return 1;
            }
        }
    } else {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            eprintln!("error: no input. Provide --text, --file, or pipe stdin.");
            return 2;
        }
        match std::io::read_to_string(std::io::stdin()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: stdin: {e}");
                return 1;
            }
        }
    };

    // Model binding: CLI --model > routing config > error.
    let model = match model_arg.or_else(|| spec.model.clone()) {
        Some(m) => m,
        None => {
            eprintln!("error: task '{name}' has no model bound. Set one in the routing config or pass --model.");
            return 1;
        }
    };
    let path = match resolve_model_path(&model) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let prompt = spec.render(&input);
    let limit = max_tokens.unwrap_or(spec.max_tokens);

    let mut backend = match aios_inference::CandleBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: backend init: {e}");
            return 1;
        }
    };
    if let Err(e) = backend.load_model(&path) {
        eprintln!("error: load {path}: {e}");
        return 1;
    }
    match backend.generate(&prompt, limit) {
        Ok(output) => {
            let tps = backend.tokens_per_second();
            if json {
                println!("{}", tasks::task_json(name, &model, &input, &output, tps));
            } else {
                println!("{output}");
                eprintln!("==> task={name} model={model} {:.2} tokens/s", tps);
            }
            0
        }
        Err(e) => {
            eprintln!("error: generate: {e}");
            1
        }
    }
}

fn cmd_doctor() -> i32 {
    fn present(name: &str, path: &str) {
        let ok = std::process::Command::new(path)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        println!(
            "  {name:12} {path:<24} [{}]",
            if ok { "OK" } else { "missing" }
        );
    }

    println!("AIOS doctor");
    println!("\nToolchain:");
    present("rustc", "rustc");
    present("cargo", "cargo");
    present("gcc", "gcc");

    println!("\nCache:");
    let models_dir = default_models_dir();
    let dir = Path::new(&models_dir);
    println!(
        "  models dir {models_dir:24} [{}]",
        if dir.is_dir() { "OK" } else { "missing" }
    );
    match find_models(dir) {
        Ok(list) => println!("  found {} model(s)", list.len()),
        Err(e) => println!("  error scanning: {e}"),
    }

    println!("\nFilesystem:");
    match std::thread::available_parallelism() {
        Ok(n) => println!("  parallelism:           {}", n.get()),
        Err(e) => println!("  parallelism:           <n/a: {e}>"),
    }
    0
}

fn type_name(vt: ValueType) -> String {
    match vt {
        ValueType::Array => "array".to_string(),
        ValueType::Bool => "bool".to_string(),
        ValueType::Float32 => "f32".to_string(),
        ValueType::Float64 => "f64".to_string(),
        ValueType::Int8 => "i8".to_string(),
        ValueType::Int16 => "i16".to_string(),
        ValueType::Int32 => "i32".to_string(),
        ValueType::Int64 => "i64".to_string(),
        ValueType::Uint8 => "u8".to_string(),
        ValueType::Uint16 => "u16".to_string(),
        ValueType::Uint32 => "u32".to_string(),
        ValueType::Uint64 => "u64".to_string(),
        ValueType::String => "string".to_string(),
        ValueType::Unknown(t) => format!("type{t}"),
    }
}
