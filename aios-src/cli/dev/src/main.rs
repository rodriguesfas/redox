//! `aios-dev` binary: argument parsing over the [`aios_dev`] library.
//!
//! Subcommands:
//!   new <name> [--template T]   scaffold a project with project.toml
//!   build [--release] [--target T]  build (in the project directory)
//!   run [--bin NAME]            run the project
//!   test                        run the project tests
//!   check                       validate project.toml, then cargo check
//!   fmt                         format the project
//!   doctor                      full environment diagnostics
//!
//! `build`/`run`/`test`/`fmt` operate on `--project DIR` (default: cwd).

use std::path::PathBuf;
use std::process::{exit, Command};

use aios_dev::project::ProjectManifest;
use aios_dev::scaffold::{self, resolve_workspace};
use aios_dev::TEMPLATES;

const USAGE: &str = "\
usage:
  aios-dev new <name> [--template TEMPLATE]
  aios-dev build [--release] [--target TARGET] [--project DIR]
  aios-dev run [--bin NAME] [--project DIR]
  aios-dev test [--project DIR]
  aios-dev check [--project DIR]
  aios-dev fmt [--project DIR]
  aios-dev doctor
  aios-dev help

templates:
  minimal       minimal Rust crate with aios-core
  edge-service  edge daemon template, isolated by default
  agent         task-based agent with AITask wiring
  classifier    text classification pipeline

environment:
  AIOS_WORKSPACE   explicit path to the AIOS workspace (used to resolve the
                   in-tree crates that generated projects depend on)
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None => {
            eprintln!("{USAGE}");
            2
        }
        Some("new") => cmd_new(&args[1..]),
        Some("build") => cmd_cargo(&args[1..], CargoAction::Build),
        Some("run") => cmd_cargo(&args[1..], CargoAction::Run),
        Some("test") => cmd_cargo(&args[1..], CargoAction::Test),
        Some("check") => cmd_check(&args[1..]),
        Some("fmt") => cmd_cargo(&args[1..], CargoAction::Fmt),
        Some("doctor") => cmd_doctor(),
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

fn cmd_new(args: &[String]) -> i32 {
    if args.is_empty() || args[0].starts_with('-') {
        eprintln!("usage: aios-dev new <name> [--template TEMPLATE]");
        return 2;
    }

    let name = args[0].clone();
    let mut template = "minimal".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--template" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--template requires a value ({})", TEMPLATES.join(", "));
                    return 2;
                }
                template = args[i].clone();
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let parent = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot determine current directory: {e}");
            return 1;
        }
    };

    match scaffold::scaffold(&parent, &name, &template) {
        Ok(result) => {
            println!(
                "Scaffolded AIOS project '{}' (template: {template})",
                result.project_dir.display()
            );
            for file in &result.files {
                println!("  {}", file.display());
            }
            println!("\nNext steps:");
            println!("  cd {name} && aios-dev check");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CargoAction {
    Build,
    Run,
    Test,
    Check,
    Fmt,
}

fn cmd_cargo(args: &[String], action: CargoAction) -> i32 {
    let mut release = false;
    let mut target: Option<String> = None;
    let mut bin: Option<String> = None;
    let mut project: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--release" => release = true,
            "--target" | "--bin" | "--project" => {
                let flag = args[i].clone();
                i += 1;
                if i >= args.len() {
                    eprintln!("{flag} requires a value");
                    return 2;
                }
                match flag.as_str() {
                    "--target" => target = Some(args[i].clone()),
                    "--bin" => bin = Some(args[i].clone()),
                    _ => project = Some(PathBuf::from(&args[i])),
                }
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let dir = match project {
        Some(p) => p,
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("cannot determine current directory: {e}");
                return 1;
            }
        },
    };

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&dir);
    match action {
        CargoAction::Build => {
            cmd.arg("build");
            if release {
                cmd.arg("--release");
            }
            if let Some(t) = &target {
                cmd.args(["--target", t]);
            }
        }
        CargoAction::Run => {
            cmd.arg("run");
            if let Some(b) = &bin {
                cmd.args(["--bin", b]);
            }
            if release {
                cmd.arg("--release");
            }
        }
        CargoAction::Test => {
            cmd.arg("test");
        }
        CargoAction::Check => {
            cmd.arg("check");
        }
        CargoAction::Fmt => {
            cmd.args(["fmt", "--all"]);
        }
    }

    let shown: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    println!("running: cargo {} (in {})", shown.join(" "), dir.display());
    match cmd.status() {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            eprintln!("cargo exited with {:?}", status.code());
            1
        }
        Err(e) => {
            eprintln!("error executing cargo: {e}");
            1
        }
    }
}

/// `check` validates `project.toml` first, then defers to `cargo check`.
///
/// The manifest check is the part that catches AIOS-specific mistakes (a
/// relative permission path, an unknown template, a malformed version), which
/// cargo would happily accept because `project.toml` is not cargo's file.
fn cmd_check(args: &[String]) -> i32 {
    let mut project: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--project requires a value");
                    return 2;
                }
                project = Some(PathBuf::from(&args[i]));
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let dir = match project {
        Some(p) => p,
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("cannot determine current directory: {e}");
                return 1;
            }
        },
    };

    let manifest_path = dir.join("project.toml");
    let mut ok = true;

    if manifest_path.is_file() {
        println!("==> project.toml");
        match ProjectManifest::load_validated(&manifest_path) {
            Ok(manifest) => {
                println!("  valid: {} ({})", manifest.project.name, manifest.target());
                if let Some(task) = &manifest.task {
                    println!("  task:  {}", task.name);
                }
                if manifest.permissions.is_some() {
                    println!("  permissions: declared");
                }
            }
            Err(errors) => {
                for error in &errors {
                    println!("  invalid: {error}");
                }
                ok = false;
            }
        }
    } else {
        println!("==> project.toml");
        println!("  absent at {}", manifest_path.display());
    }

    // Only run cargo when the manifest is sound; a broken manifest usually
    // means the crate itself is not what the author thinks it is.
    //
    // Forward the directory. Without this, `dev check --project ../other` would
    // validate `../other/project.toml` and then run `cargo check` against the
    // *current* directory — a green "check: OK" for the wrong crate.
    if ok {
        let dir_arg = dir.display().to_string();
        let cargo_args = ["--project".to_string(), dir_arg];
        if cmd_cargo(&cargo_args, CargoAction::Check) != 0 {
            ok = false;
        }
    }

    if ok {
        println!("\ncheck: OK");
        0
    } else {
        println!("\ncheck: FAILED");
        1
    }
}
fn cmd_doctor() -> i32 {
    println!("AIOS Developer Doctor");
    println!("=====================");
    println!("models dir: {}", aios_dev::doctor::models_dir().display());
    match resolve_workspace() {
        Some(ws) => println!("workspace:  {}", ws.display()),
        None => println!("workspace:  NOT FOUND (set AIOS_WORKSPACE)"),
    }

    let report = aios_dev::doctor::run_with_workspace_lookup();
    print!("{report}");

    println!();
    if report.is_healthy() {
        println!("==> Environment ready for AIOS development.");
        0
    } else {
        println!("==> Components require attention:");
        for failure in report.failures() {
            println!("  - {}: {}", failure.name, failure.detail);
        }
        1
    }
}
