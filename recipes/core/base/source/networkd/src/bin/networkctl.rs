//! `networkctl`: inspect and apply the network configuration.
//!
//! The configuration is a directory of `*.network` files; the network is the
//! file tree the stack serves at `/scheme/netcfg`. `list` and `status` read
//! both and report; `show` reads the configuration alone; `apply` is the only
//! command that writes to the stack, and `--dry-run` makes it a `show` of the
//! writes instead.
//!
//! Nothing is written unless asked: a boot with no configuration is a boot
//! where every interface is left to DHCP, which is what the system does today.

use std::process::ExitCode;

use networkd::cli::parse_env_args;
use networkd::config::{Config, NetworkFile};
use networkd::netcfg::Netcfg;
use networkd::plan::Plan;
use networkd::render::{render_applied, render_config, render_interfaces, render_status};
use networkd::{Command, USAGE};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cli = match parse_env_args(args) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("networkctl: {err}");
            eprintln!("Try 'networkctl --help' for more information.");
            return ExitCode::FAILURE;
        }
    };

    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if cli.version {
        println!("networkctl {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("networkctl: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &networkd::Cli) -> Result<(), String> {
    match cli.command {
        Command::Show => {
            let config = match &cli.target {
                Some(path) => load_config_file(path)?,
                None => load_config(cli)?,
            };
            print!("{}", render_config(&config));
            report_warnings(&config.warnings());
            Ok(())
        }
        Command::List => {
            let plan = build_plan(cli)?;
            print!("{}", render_interfaces(&plan, cli.no_legend));
            report_warnings(&plan.warnings);
            Ok(())
        }
        Command::Status => {
            let plan = build_plan(cli)?;
            let root = cli.netcfg.to_string_lossy().into_owned();

            if let Some(name) = &cli.target {
                if !plan.interfaces.iter().any(|iface| &iface.name == name) {
                    return Err(networkd::render::unknown_interface(name, &plan)
                        .trim_end()
                        .to_owned());
                }
            }

            print!("{}", render_status(&plan, cli.target.as_deref(), &root));
            report_warnings(&plan.warnings);
            Ok(())
        }
        Command::Apply => {
            let plan = build_plan(cli)?;
            let netcfg = Netcfg::new(cli.netcfg.to_string_lossy().into_owned());
            let writes = plan.writes();
            report_warnings(&plan.warnings);

            if writes.is_empty() {
                println!("networkctl: nothing to apply");
                return Ok(());
            }

            let rendered: Vec<String> = writes
                .iter()
                .map(|write| write.render(netcfg.root()))
                .collect();

            if cli.dry_run {
                print!("{}", render_applied(&rendered));
                return Ok(());
            }

            let applied = netcfg.apply(&writes).map_err(|err| err.0)?;
            print!("{}", render_applied(&applied));
            Ok(())
        }
        Command::None => Err("no command given".to_owned()),
    }
}

/// The configuration of the system: the directory of `*.network` files.
fn load_config(cli: &networkd::Cli) -> Result<Config, String> {
    Config::load(&cli.config_dir).map_err(|err| err.0)
}

/// One configuration file, which is what `show FILE` reads.
fn load_config_file(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("cannot read {path}: {err}"))?;
    let file = NetworkFile::parse(&text, Some(path.into())).map_err(|err| err.0)?;
    Ok(Config::from_files(vec![file]))
}

fn build_plan(cli: &networkd::Cli) -> Result<Plan, String> {
    let config = load_config(cli)?;
    let netcfg = Netcfg::new(cli.netcfg.to_string_lossy().into_owned());
    let observed = netcfg.interfaces().map_err(|err| err.0)?;

    Ok(Plan::build(&config, &observed))
}

fn report_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("networkctl: warning: {warning}");
    }
}
