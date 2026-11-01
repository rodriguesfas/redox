//! `aios-deploy`: Edge Deployment CLI (Fase 7).
//!
//!   aios-deploy devices | list       list registered devices
//!   aios-deploy add <name> <addr>    register a new edge device
//!   aios-deploy remove <id>          remove a device
//!   aios-deploy deploy <id> <model>  deploy a model to a device
//!   aios-deploy status <id>          show deployment status (live probe)
//!   aios-deploy rollback <id> <model> restore the previous model version
//!   aios-deploy validate <id> <model> validate compatibility
//!   aios-deploy scan [prefix]        discover edge-ai nodes on a /24 LAN
//!   aios-deploy provision <addr>     probe + register a node (RJ45 onboarding)
//!   aios-deploy probe <addr>         live health without registering
//!   aios-deploy help

use aios_deploy::{
    normalize_base_url, probe_device, scan_subnet, validate_deployment, DeviceEntry, DeviceRegistry,
    DeviceStatus, TransferConfig,
};
use std::path::PathBuf;
use std::process::exit;

const USAGE: &str = "\
usage:
  aios-deploy devices | list
  aios-deploy add <name> <address> [--arch ARCH] [--os OS] [--ram MB] [--storage MB]
  aios-deploy remove <id>
  aios-deploy deploy <device-id> <model-path> [--name NAME] [--force] [--no-verify]
  aios-deploy status <device-id>
  aios-deploy rollback <device-id> <model-name>
  aios-deploy validate <device-id> <model-path>
  aios-deploy scan [prefix]          # default prefix 10.42.0 (PC↔Pi cabo)
  aios-deploy provision <address> [--name NAME]
  aios-deploy probe <address>
  aios-deploy help";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        None => {
            eprintln!("{USAGE}");
            2
        }
        Some("devices" | "list") => cmd_devices(&args[1..]),
        Some("add") => cmd_add(&args[1..]),
        Some("remove") => cmd_remove(&args[1..]),
        Some("deploy") => cmd_deploy(&args[1..]),
        Some("status") => cmd_status(&args[1..]),
        Some("rollback") => cmd_rollback(&args[1..]),
        Some("validate") => cmd_validate(&args[1..]),
        Some("scan") => cmd_scan(&args[1..]),
        Some("provision") => cmd_provision(&args[1..]),
        Some("probe") => cmd_probe(&args[1..]),
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

fn cmd_devices(_args: &[String]) -> i32 {
    let reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    if reg.is_empty() {
        println!("No devices registered.");
        println!("Use 'aios-deploy add <name> <address>' to register a device.");
        return 0;
    }

    println!("Registered devices ({}):", reg.len());
    for device in reg.list() {
        println!(
            "  {}  {}  {}  {}MB RAM  {}MB storage  {}  models={}",
            &device.id[..8.min(device.id.len())],
            device.name,
            device.address,
            device.ram_mb,
            device.storage_mb,
            device.status_str(),
            device.models.len()
        );
    }
    0
}

/// Find a device by full id, id prefix, or name.
///
/// `devices` prints an 8-character id prefix, so accepting only the full UUID
/// meant the value the CLI showed was not a value it accepted. A name has to
/// work too: it is what an operator naturally types, and the ROADMAP's example
/// is `edge deploy <model.gguf> --device <id>`.
fn resolve_device<'a>(reg: &'a DeviceRegistry, needle: &str) -> Option<&'a DeviceEntry> {
    if let Some(device) = reg.get(needle) {
        return Some(device);
    }
    // Unique id prefix.
    let prefix_matches: Vec<&DeviceEntry> = reg
        .list()
        .into_iter()
        .filter(|d| d.id.starts_with(needle))
        .collect();
    let by_prefix = match prefix_matches.as_slice() {
        [only] => Some(*only),
        [] => None,
        many => {
            eprintln!(
                "Device id prefix '{needle}' is ambiguous ({} matches: {})",
                many.len(),
                many.iter()
                    .map(|d| format!("{} ({})", d.name, &d.id[..8]))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return None;
        }
    };
    by_prefix.or_else(|| reg.find_by_name(needle))
}

fn cmd_add(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: aios-deploy add <name> <address> [--arch ARCH] [--os OS] [--ram MB] [--storage MB]");
        return 2;
    }

    let name = args[0].clone();
    let address = args[1].clone();

    let mut arch = "aarch64".to_string();
    let mut os = "redox".to_string();
    let mut ram_mb = 2048u64;
    let mut storage_mb = 8192u64;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--arch" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--arch requires a value");
                    return 2;
                }
                arch = args[i].clone();
            }
            "--os" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--os requires a value");
                    return 2;
                }
                os = args[i].clone();
            }
            "--ram" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--ram requires a value");
                    return 2;
                }
                ram_mb = args[i].parse().unwrap_or(2048);
            }
            "--storage" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--storage requires a value");
                    return 2;
                }
                storage_mb = args[i].parse().unwrap_or(8192);
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let mut reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let mut entry = DeviceEntry::new(name.clone(), address.clone(), arch, os);
    entry.ram_mb = ram_mb;
    entry.storage_mb = storage_mb;

    let id = reg.add(entry);
    if let Err(e) = reg.save() {
        eprintln!("error saving registry: {e}");
        return 1;
    }

    println!("Added device '{name}' ({id}) at {address}");
    0
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(id) = args.first() else {
        eprintln!("usage: aios-deploy remove <id>");
        return 2;
    };

    let mut reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    if reg.remove(id).is_some() {
        if let Err(e) = reg.save() {
            eprintln!("error saving registry: {e}");
            return 1;
        }
        println!("Removed device {id}");
        0
    } else {
        eprintln!("Device {id} not found");
        1
    }
}

fn cmd_validate(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: aios-deploy validate <device-id> <model-path>");
        return 2;
    }

    let device_id = &args[0];
    let model_path = PathBuf::from(&args[1]);

    let reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    // Accept either the id or the name. `devices` prints an 8-character id
    // prefix, so requiring the full UUID meant the only value the CLI shows
    // you was not the value it would accept.
    let device = match resolve_device(&reg, device_id) {
        Some(d) => d,
        None => {
            eprintln!("Device {device_id} not found");
            eprintln!("Run 'aios-deploy devices' to list registered devices.");
            return 1;
        }
    };

    match validate_deployment(device, &model_path) {
        Ok(check) => {
            println!(
                "Compatibility Check for '{}' on '{}':",
                model_path.display(),
                device.name
            );
            println!("  Architecture match:     {}", check.arch_match);
            println!(
                "  RAM sufficient:         {} (need {}MB, have {}MB)",
                check.ram_sufficient, check.required_ram_mb, check.available_ram_mb
            );
            println!(
                "  Storage sufficient:     {} (need {}MB, have {}MB)",
                check.storage_sufficient, check.required_storage_mb, check.available_storage_mb
            );
            println!("  Model exists:           {}", check.model_exists);
            println!(
                "  OVERALL:                {}",
                if check.compatible {
                    "COMPATIBLE"
                } else {
                    "INCOMPATIBLE"
                }
            );

            for w in &check.warnings {
                println!("  WARNING: {w}");
            }
            for e in &check.errors {
                println!("  ERROR:   {e}");
            }

            if check.compatible {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("Validation error: {e}");
            1
        }
    }
}

fn cmd_deploy(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: aios-deploy deploy <device-id> <model-path> [--name NAME] [--force] [--no-verify]");
        return 2;
    }

    let device_id = &args[0];
    let model_path = PathBuf::from(&args[1]);

    let mut model_name = None;
    let mut force = false;
    let mut verify_checksum = true;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--name requires a value");
                    return 2;
                }
                model_name = Some(args[i].clone());
            }
            "--force" => force = true,
            "--no-verify" => verify_checksum = false,
            "--no-start" => {
                // The daemon has no service-control route: /api/health,
                // /api/models, /api/infer, /api/logs, /api/metrics and the two
                // upload routes are the whole surface. There is no endpoint to
                // start, stop or reload a model, so there is nothing for this
                // flag to switch off. Accepting it silently would tell an
                // operator it did something it cannot do.
                eprintln!("error: --no-start is not supported.");
                eprintln!(
                    "       the edge daemon exposes no service-control endpoint, so the client \
                     cannot start or stop anything remotely. Service lifecycle on the device \
                     (systemd unit, init script) has to be managed on the device itself."
                );
                return 2;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let device = match resolve_device(&reg, device_id) {
        Some(d) => d,
        None => {
            eprintln!("Device {device_id} not found");
            eprintln!("Run 'aios-deploy devices' to list registered devices.");
            return 1;
        }
    };

    // Snapshot what the registry update below needs, so `device`'s borrow of
    // `reg` can end before the registry is written.
    let resolved_id = device.id.clone();
    let resolved_models = device.models.clone();

    println!("Deploying model to {} ({})...", device.name, device.address);

    let name = model_name.unwrap_or_else(|| aios_core::default_name_for(&model_path));

    // Validate first
    match validate_deployment(device, &model_path) {
        Ok(check) => {
            if !check.compatible && !force {
                eprintln!("Deployment validation failed (use --force to override):");
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
            eprintln!("Validation error: {e}");
            return 1;
        }
    }

    // The ROADMAP step order is: validate -> compat -> storage -> transfer ->
    // install -> configure -> start -> health. Transfer is where the install
    // actually happens: the edge daemon assembles the chunks, verifies the
    // SHA-256, writes the file into its models dir and registers it. So a
    // successful finalize *is* the install step, and the health check below
    // confirms the daemon came back with the model loaded.
    let config = TransferConfig {
        base_url: format!("http://{}", device.address),
        verify_checksum,
        ..Default::default()
    };

    let result = match aios_deploy::transfer_model(&config, &device.id, &model_path, &name) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Deployment failed: {e}");
            return 1;
        }
    };

    println!("Transfer complete:");
    println!("  Model:        {}", result.model_name);
    println!("  Size:         {} bytes", result.model_size);
    println!("  SHA-256:      {}", result.sha256);
    println!("  Transferred:  {} bytes", result.bytes_transferred);
    println!("  Attempts:     {}", result.attempts);

    if !verify_checksum {
        // Be explicit rather than silently permissive. The edge daemon
        // verifies the SHA-256 itself in every case, so this flag cannot make
        // a corrupt model install; what it changes is the client's own
        // pre-flight check. Saying so keeps the flag honest.
        println!(
            "\n  note: --no-verify skipped the client's post-transfer re-read of the model file."
        );
        println!("        the edge daemon still recomputes the SHA-256 of what it received and");
        println!("        rejects a mismatch, so this flag cannot install a corrupt model.");
    }

    // Record the outcome on the device so `devices` reflects reality. Without
    // this the registry keeps reporting the pre-deploy state: status stuck at
    // "unknown" and models=0 even after a model landed, which reads as if the
    // deploy did nothing. Re-load rather than mutating the borrowed `reg`, which
    // `device` still borrows from.
    let mut model_list = resolved_models;
    if !model_list.contains(&result.model_name) {
        model_list.push(result.model_name.clone());
    }
    match DeviceRegistry::load() {
        Ok(mut reg) => {
            reg.update_status(&resolved_id, DeviceStatus::Online);
            reg.update_models(&resolved_id, model_list);
            reg.touch(&resolved_id);
            if let Err(e) = reg.save() {
                eprintln!("warning: could not update the device registry: {e}");
            }
        }
        Err(e) => eprintln!("warning: could not re-read the registry to record the deploy: {e}"),
    }

    // Health check. A transfer that succeeded but left the daemon unable to
    // serve the model is not a successful deployment, so this is part of the
    // deploy, not an optional extra.
    println!("\nHealth check:");
    match aios_deploy::health_check(&config.base_url, &result.model_name) {
        Ok(health) => {
            println!("  service:   {}", health.service);
            println!("  status:    {}", health.status);
            println!("  version:   {}", health.version);
            println!("  model:     {} loaded", health.model);
            0
        }
        Err(e) => {
            eprintln!("  FAILED: {e}");
            eprintln!(
                "\nThe model transferred and installed, but the device did not come \
                 back healthy. It may need a service restart, or the model may be \
                 too large to load on this device."
            );

            // The daemon keeps the replaced model under `.backup/`. Restoring
            // it is what makes a failed deploy safe: the device goes back to
            // serving the version it had before instead of staying stuck on a
            // model it cannot load.
            eprintln!("\nRolling back to the previous version...");
            match aios_deploy::rollback_model(&config.base_url, &result.model_name) {
                Ok(r) => {
                    eprintln!("  rolled back to '{}'", r.model);
                    eprintln!("  sha256:        {}", r.sha256);
                    eprintln!("  size:          {} bytes", r.size_bytes);
                    match aios_deploy::probe_device(&config.base_url) {
                        Ok(p) => eprintln!(
                            "  device now:    {} ({} model(s) installed)",
                            p.status,
                            p.models.len()
                        ),
                        Err(pe) => eprintln!("  warning: could not re-probe the device: {pe}"),
                    }
                }
                Err(re) => {
                    eprintln!("  rollback failed: {re}");
                    eprintln!(
                        "  the device may be left with a model it cannot serve; \
                         inspect it with 'aios-deploy status'."
                    );
                    // `devices` was updated to Online before the health check.
                    // Record the failure so it does not keep claiming the
                    // device is fine.
                    if let Ok(mut reg) = DeviceRegistry::load() {
                        reg.update_status(
                            &resolved_id,
                            DeviceStatus::Error("deploy health check failed".into()),
                        );
                        let _ = reg.save();
                    }
                }
            }
            1
        }
    }
}

fn cmd_status(args: &[String]) -> i32 {
    if args.len() != 1 {
        eprintln!("usage: aios-deploy status <device-id>");
        return 2;
    }

    let reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };
    let device = match resolve_device(&reg, &args[0]) {
        Some(d) => d,
        None => {
            eprintln!("Device {} not found", args[0]);
            eprintln!("Run 'aios-deploy devices' to list registered devices.");
            return 1;
        }
    };

    // Snapshot the fields before dropping the registry borrow, because the
    // probe below updates and saves the same registry.
    let id = device.id.clone();
    let name = device.name.clone();
    let address = device.address.clone();
    let arch = device.arch.clone();
    let os = device.os.clone();
    let ram_mb = device.ram_mb;
    let storage_mb = device.storage_mb;
    let registry_status = device.status_str().to_string();
    let models = device.models.clone();
    let last_seen = device.last_seen;

    println!("Device {name}");
    println!("  id:        {id}");
    println!("  address:   {address}");
    println!("  arch/os:   {arch} / {os}");
    println!("  ram:       {ram_mb} MB");
    println!("  storage:   {storage_mb} MB");
    println!("  registry:  {registry_status}");
    println!(
        "  last seen: {}",
        if last_seen == 0 {
            "never".to_string()
        } else {
            format!("{last_seen} (unix)")
        }
    );
    if models.is_empty() {
        println!("  models:    <none>");
    } else {
        println!("  models:    {}", models.join(", "));
    }

    println!("\nLive probe:");
    let base = format!("http://{address}");
    match aios_deploy::probe_device(&base) {
        Ok(p) => {
            println!("  service:   {} v{}", p.service, p.version);
            println!("  status:    {}", p.status);
            if p.models.is_empty() {
                println!("  models:    <none>");
            } else {
                println!("  models:    {}", p.models.join(", "));
            }
            if let Ok(mut reg) = DeviceRegistry::load() {
                reg.update_status(&id, DeviceStatus::Online);
                reg.update_models(&id, p.models);
                reg.touch(&id);
                if let Err(e) = reg.save() {
                    eprintln!("  warning: could not update the device registry: {e}");
                }
            }
            0
        }
        Err(e) => {
            println!("  unreachable: {e}");
            if let Ok(mut reg) = DeviceRegistry::load() {
                reg.update_status(&id, DeviceStatus::Offline);
                if let Err(e) = reg.save() {
                    eprintln!("  warning: could not update the device registry: {e}");
                }
            }
            1
        }
    }
}

fn cmd_scan(args: &[String]) -> i32 {
    let prefix = args.first().map(|s| s.as_str()).unwrap_or("10.42.0");
    // Allow "10.42.0.0/24" or "10.42.0." as well as "10.42.0".
    let prefix = prefix
        .trim_end_matches('/')
        .trim_end_matches("24")
        .trim_end_matches('/')
        .trim_end_matches('.')
        .to_string();
    let parts: Vec<&str> = prefix.split('.').collect();
    let prefix = if parts.len() >= 3 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        prefix
    };

    println!("Scanning {prefix}.0/24 for edge-ai on :8989 …");
    let hits = scan_subnet(&prefix, 8989);
    if hits.is_empty() {
        println!("No AIOS Edge nodes found.");
        println!("Check: cabo RJ45, GENET/DHCP no Pi, edge-ai a escutar.");
        return 1;
    }
    println!("Found {} node(s):", hits.len());
    for (addr, probe) in &hits {
        println!(
            "  {addr}  {} v{}  status={}  models={}",
            probe.service,
            probe.version,
            probe.status,
            if probe.models.is_empty() {
                "<none>".to_string()
            } else {
                probe.models.join(",")
            }
        );
    }
    println!("Next: aios-deploy provision <address>");
    0
}

fn cmd_probe(args: &[String]) -> i32 {
    let Some(address) = args.first() else {
        eprintln!("usage: aios-deploy probe <address>");
        return 2;
    };
    let base = normalize_base_url(address);
    match probe_device(&base) {
        Ok(p) => {
            println!("probe {base}");
            println!("  service: {}", p.service);
            println!("  version: {}", p.version);
            println!("  status:  {}", p.status);
            println!(
                "  models:  {}",
                if p.models.is_empty() {
                    "<none>".to_string()
                } else {
                    p.models.join(", ")
                }
            );
            0
        }
        Err(e) => {
            eprintln!("probe failed: {e}");
            1
        }
    }
}

fn cmd_provision(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: aios-deploy provision <address> [--name NAME]");
        return 2;
    }
    let address = args[0].clone();
    let mut name = format!(
        "pi-{}",
        address
            .split([':', '/'])
            .next()
            .unwrap_or("edge")
            .rsplit('.')
            .next()
            .unwrap_or("edge")
    );
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--name requires a value");
                    return 2;
                }
                name = args[i].clone();
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let base = normalize_base_url(&address);
    let probe = match probe_device(&base) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("provision: device not reachable at {base}: {e}");
            return 1;
        }
    };
    if probe.service != "edge-ai" && probe.status != "ok" {
        eprintln!(
            "provision: unexpected service '{}' status '{}'",
            probe.service, probe.status
        );
        return 1;
    }

    // Store host:port without scheme — same shape as `add` / `status`.
    let registry_addr = base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();

    let mut reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    // Update existing by name or address; otherwise add.
    let existing_id = reg
        .list()
        .into_iter()
        .find(|d| d.name == name || d.address == registry_addr)
        .map(|d| d.id.clone());

    let id = if let Some(id) = existing_id {
        if let Some(entry) = reg.get(&id).cloned() {
            let mut entry = entry;
            entry.address = registry_addr.clone();
            entry.name = name.clone();
            entry.arch = "aarch64".into();
            entry.os = "aios-edge".into();
            entry.status = DeviceStatus::Online;
            entry.models = probe.models.clone();
            reg.upsert(entry);
        }
        id
    } else {
        let mut entry = DeviceEntry::new(
            name.clone(),
            registry_addr.clone(),
            "aarch64".into(),
            "aios-edge".into(),
        );
        entry.status = DeviceStatus::Online;
        entry.models = probe.models.clone();
        entry.ram_mb = 4096;
        entry.storage_mb = 8192;
        reg.add(entry)
    };

    if let Err(e) = reg.save() {
        eprintln!("error saving registry: {e}");
        return 1;
    }

    println!("Provisioned '{name}' ({id}) at {registry_addr}");
    println!(
        "  {} v{}  models={}",
        probe.service,
        probe.version,
        if probe.models.is_empty() {
            "<none>".to_string()
        } else {
            probe.models.join(",")
        }
    );
    0
}

fn cmd_rollback(args: &[String]) -> i32 {
    if args.len() != 2 {
        eprintln!("usage: aios-deploy rollback <device-id> <model-name>");
        return 2;
    }

    let reg = match DeviceRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };
    let device = match resolve_device(&reg, &args[0]) {
        Some(d) => d,
        None => {
            eprintln!("Device {} not found", args[0]);
            eprintln!("Run 'aios-deploy devices' to list registered devices.");
            return 1;
        }
    };
    let base = format!("http://{}", device.address);

    match aios_deploy::rollback_model(&base, &args[1]) {
        Ok(r) => {
            println!(
                "Rolled back '{}' on {} ({} bytes, sha256 {})",
                r.model, device.name, r.size_bytes, r.sha256
            );
            0
        }
        Err(e) => {
            eprintln!("rollback failed: {e}");
            1
        }
    }
}
