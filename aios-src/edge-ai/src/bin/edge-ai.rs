//! `edge-ai`: Edge AI OS daemon (Fase 5). Options:
//!   edge-ai [--host H] [--port P] [--model M] [--history-every S]

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut host = edge_ai::DEFAULT_BIND.to_string();
    let mut port = edge_ai::DEFAULT_PORT;
    let mut model: Option<String> = None;
    let mut history_every: u64 = 5;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" | "-H" => {
                i += 1;
                if i >= args.len() {
                    usage("--host requires a value");
                }
                host = args[i].clone();
            }
            "--port" | "-p" => {
                i += 1;
                if i >= args.len() {
                    usage("--port requires a number");
                }
                match args[i].parse() {
                    Ok(p) => port = p,
                    Err(_) => usage("--port expects a number"),
                }
            }
            "--model" | "-m" => {
                i += 1;
                if i >= args.len() {
                    usage("--model requires a value");
                }
                model = Some(args[i].clone());
            }
            "--history-every" => {
                i += 1;
                if i >= args.len() {
                    usage("--history-every requires a number");
                }
                match args[i].parse() {
                    Ok(s) => history_every = s,
                    Err(_) => usage("--history-every expects a number"),
                }
            }
            "help" | "-h" | "--help" => {
                println!("usage: edge-ai [--host H] [--port P] [--model M] [--history-every S]");
                exit(0);
            }
            other => usage(&format!("unexpected argument: {other}")),
        }
        i += 1;
    }

    if let Err(e) = edge_ai::daemon::run(&host, port, model.as_deref(), history_every) {
        eprintln!("edge-ai: {e}");
        exit(1);
    }
}

fn usage(msg: &str) -> ! {
    eprintln!("edge-ai: {msg}");
    eprintln!("usage: edge-ai [--host H] [--port P] [--model M] [--history-every S]");
    exit(2);
}
