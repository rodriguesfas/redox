//! `aios-security`: AI Model Security CLI (Fase 8).
//!
//!   aios-security policies              list all policies
//!   aios-security add <name>           create a new policy (interactive or --template)
//!   aios-security remove <name>        remove a policy
//!   aios-security show <name>          show policy details
//!   aios-security create-ai-model <model>  create restrictive policy for AI model
//!   aios-security create-dev <name>    create permissive dev policy
//!   aios-security check <policy> <resource> <access> [--scope]  check permission
//!   aios-security authorize <policy> <resource> <access> [--scope P] [--context k=v]
//!   aios-security describe <policy>          show the rule set in evaluation order
//!   aios-security contain-profiles           list contain profiles
//!   aios-security contain-plan <profile>     show the isolation a profile grants
//!   aios-security help
//!
//! `check` prints the raw policy effect. `authorize` runs the same request
//! through the enforcement layer: it resolves the deciding rule, reports the
//! audit entry, and fails closed on a malformed request.
//!
//! `contain-plan` is a *description* of isolation, not a running container:
//! creating a container still needs the Redox `contain` scheme, which is not
//! implemented (see docs/gotchas/security-isolation.md).

use aios_security::contain::ContainManager;
use aios_security::enforce::AccessRequest;
use aios_security::permissions::{AccessLevel, ResourceType};
use aios_security::policy::{PolicyEffect, PolicyRule};
use aios_security::{AIPermissions, Enforcer, PolicyRegistry, SecurityPolicy};
use std::process::exit;

const USAGE: &str = "\
usage:
  aios-security policies
  aios-security add <name> [--template TEMPLATE]
  aios-security remove <name>
  aios-security show <name>
  aios-security create-ai-model <model-name>
  aios-security create-dev <name>
  aios-security check <policy> <resource> <access> [--scope PATH]
  aios-security authorize <policy> <resource> <access> [--scope PATH] [--context k=v]
  aios-security describe <policy>
  aios-security contain-profiles
  aios-security contain-plan <profile> [name]
  aios-security help";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        None => {
            eprintln!("{USAGE}");
            2
        }
        Some("policies") => cmd_policies(),
        Some("add") => cmd_add(&args[1..]),
        Some("remove") => cmd_remove(&args[1..]),
        Some("show") => cmd_show(&args[1..]),
        Some("create-ai-model") => cmd_create_ai_model(&args[1..]),
        Some("create-dev") => cmd_create_dev(&args[1..]),
        Some("check") => cmd_check(&args[1..]),
        Some("authorize") => cmd_authorize(&args[1..]),
        Some("describe") => cmd_describe(&args[1..]),
        Some("contain-profiles") => cmd_contain_profiles(),
        Some("contain-plan") => cmd_contain_plan(&args[1..]),
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

fn cmd_policies() -> i32 {
    let reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    if reg.is_empty() {
        println!("No policies registered.");
        println!("Use 'aios-security create-ai-model <model>' or 'create-dev <name>' to create policies.");
        return 0;
    }

    println!("Registered policies ({}):", reg.len());
    for policy in reg.list() {
        println!("  {}", policy.summary());
    }
    0
}

fn cmd_add(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: aios-security add <name> [--template TEMPLATE]");
        return 2;
    }

    let name = &args[0];
    let mut template = "empty";

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--template" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--template requires a value");
                    return 2;
                }
                template = &args[i];
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let mut reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let mut policy = SecurityPolicy::new(name);
    policy.default_effect = PolicyEffect::Deny;

    match template {
        "ai-model" | "edge" => {
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Filesystem,
                    AccessLevel::Read,
                )
                .with_scope("/var/lib/ai/models")
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Filesystem,
                    AccessLevel::Write,
                )
                .with_scope("/tmp/ai-inference")
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Network,
                    AccessLevel::Read,
                )
                .with_condition("port".to_string(), "8989".to_string())
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Compute,
                    AccessLevel::Execute,
                )
                .with_condition("max_memory".to_string(), "2048".to_string())
                .with_condition("max_cpu".to_string(), "80".to_string())
                .with_priority(10),
            );
        }
        "dev" => {
            policy.default_effect = PolicyEffect::Allow;
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Filesystem,
                    AccessLevel::ReadWrite,
                )
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Network,
                    AccessLevel::ReadWrite,
                )
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Device,
                    AccessLevel::Execute,
                )
                .with_priority(10),
            );
            policy.add_rule(
                PolicyRule::new(
                    PolicyEffect::Allow,
                    ResourceType::Compute,
                    AccessLevel::Execute,
                )
                .with_priority(10),
            );
        }
        _ => {}
    }

    let ai_perms = match template {
        "ai-model" | "edge" => Some(AIPermissions::edge_default(name)),
        "dev" => Some(AIPermissions::permissive(name)),
        _ => None,
    };

    let id = reg.upsert(name, policy, ai_perms);
    if let Err(e) = reg.save() {
        eprintln!("error saving registry: {e}");
        return 1;
    }

    println!("Added policy '{name}' ({id}) with template '{template}'");
    0
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: aios-security remove <name>");
        return 2;
    };

    let mut reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    if reg.remove(name).is_some() {
        if let Err(e) = reg.save() {
            eprintln!("error saving registry: {e}");
            return 1;
        }
        println!("Removed policy {name}");
        0
    } else {
        eprintln!("Policy {name} not found");
        1
    }
}

fn cmd_show(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: aios-security show <name>");
        return 2;
    };

    let reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let entry = match reg.get(name) {
        Some(e) => e,
        None => {
            eprintln!("Policy {name} not found");
            return 1;
        }
    };

    println!("Policy: {}", entry.name);
    println!("ID:     {}", entry.id);
    println!("Rules:  {}", entry.policy.rules.len());
    println!("Default: {:?}", entry.policy.default_effect);
    println!("Created: {}", entry.created_at);
    println!("Updated: {}", entry.updated_at);

    if let Some(perms) = &entry.ai_permissions {
        println!("\nAI Permissions:");
        println!("  Model:      {}", perms.model_name);
        println!("  Isolate:    {}", perms.isolate);
        println!("  Profile:    {:?}", perms.contain_profile);
        println!("  Rules:      {}", perms.permissions.rules.len());
        for rule in &perms.permissions.rules {
            println!(
                "    {:?} {:?} scope={:?}",
                rule.resource, rule.access, rule.scope
            );
        }
    }

    println!("\nPolicy Rules:");
    for rule in &entry.policy.rules {
        println!(
            "  [{:?}] {:?} {:?} {:?} scope={:?} priority={}",
            rule.effect, rule.resource, rule.access, rule.conditions, rule.scope, rule.priority
        );
    }

    0
}

fn cmd_create_ai_model(args: &[String]) -> i32 {
    let Some(model) = args.first() else {
        eprintln!("usage: aios-security create-ai-model <model-name>");
        return 2;
    };

    let mut reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let id = reg.create_ai_model_policy(model);
    if let Err(e) = reg.save() {
        eprintln!("error saving registry: {e}");
        return 1;
    }

    println!("Created AI model policy for '{model}' ({id})");
    println!("  - Restrictive: filesystem read /var/lib/ai/models, write /tmp/ai-inference");
    println!("  - Network: inbound port 8989 only");
    println!("  - Compute: max 2GB RAM, 80% CPU");
    println!("  - Isolated: yes (contain profile: ai-model)");
    0
}

fn cmd_create_dev(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: aios-security create-dev <name>");
        return 2;
    };

    let mut reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return 1;
        }
    };

    let id = reg.create_dev_policy(name);
    if let Err(e) = reg.save() {
        eprintln!("error saving registry: {e}");
        return 1;
    }

    println!("Created dev policy '{name}' ({id})");
    println!("  - Permissive: full access to all resources");
    println!("  - Isolated: no");
    0
}

fn parse_resource(s: &str) -> Result<ResourceType, i32> {
    match s.to_lowercase().as_str() {
        "filesystem" | "fs" => Ok(ResourceType::Filesystem),
        "network" | "net" => Ok(ResourceType::Network),
        "device" => Ok(ResourceType::Device),
        "compute" | "cpu" => Ok(ResourceType::Compute),
        "ipc" => Ok(ResourceType::Ipc),
        "systeminfo" | "sysinfo" => Ok(ResourceType::SystemInfo),
        _ => {
            eprintln!(
                "unknown resource: {s}\n  valid: filesystem, network, device, compute, ipc, systeminfo"
            );
            Err(2)
        }
    }
}

fn parse_access(s: &str) -> Result<AccessLevel, i32> {
    match s.to_lowercase().as_str() {
        "read" | "r" => Ok(AccessLevel::Read),
        "write" | "w" => Ok(AccessLevel::Write),
        "readwrite" | "rw" => Ok(AccessLevel::ReadWrite),
        "execute" | "exec" | "x" => Ok(AccessLevel::Execute),
        "none" => Ok(AccessLevel::None),
        _ => {
            eprintln!("unknown access level: {s}\n  valid: read, write, readwrite, execute, none");
            Err(2)
        }
    }
}

/// Load a policy by name from the registry.
fn load_policy(name: &str) -> Result<SecurityPolicy, i32> {
    let reg = match PolicyRegistry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error loading registry: {e}");
            return Err(1);
        }
    };
    match reg.get(name) {
        Some(entry) => Ok(entry.policy.clone()),
        None => {
            eprintln!("Policy {name} not found");
            eprintln!("Run 'aios-security policies' to list the available policies.");
            Err(1)
        }
    }
}

fn cmd_check(args: &[String]) -> i32 {
    if args.len() < 3 {
        eprintln!("usage: aios-security check <policy> <resource> <access> [--scope PATH]");
        return 2;
    }

    let policy_name = &args[0];
    let resource = match parse_resource(&args[1]) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let access = match parse_access(&args[2]) {
        Ok(a) => a,
        Err(code) => return code,
    };

    let mut scope = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--scope" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--scope requires a value");
                    return 2;
                }
                scope = Some(args[i].clone());
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let policy = match load_policy(policy_name) {
        Ok(p) => p,
        Err(code) => return code,
    };

    let context = std::collections::HashMap::new();
    let effect = policy.evaluate(resource, access, scope.as_deref(), &context);

    println!("Policy: {policy_name}");
    println!("Request: {resource:?} {access:?} scope={scope:?}");
    println!("Decision: {effect:?}");

    if effect == PolicyEffect::Allow {
        println!("✓ ALLOWED");
        0
    } else {
        println!("✗ DENIED");
        1
    }
}

fn cmd_authorize(args: &[String]) -> i32 {
    if args.len() < 3 {
        eprintln!(
            "usage: aios-security authorize <policy> <resource> <access> \
             [--scope PATH] [--context key=value]..."
        );
        return 2;
    }

    let policy_name = &args[0];
    let resource = match parse_resource(&args[1]) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let access = match parse_access(&args[2]) {
        Ok(a) => a,
        Err(code) => return code,
    };

    let mut request = AccessRequest::new(resource, access);
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--scope" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--scope requires a value");
                    return 2;
                }
                request = request.with_scope(&args[i]);
            }
            "--context" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--context requires key=value");
                    return 2;
                }
                match args[i].split_once('=') {
                    Some((k, v)) if !k.is_empty() => {
                        request = request.with_context(k, v);
                    }
                    _ => {
                        eprintln!("--context expects key=value, got: {}", args[i]);
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let policy = match load_policy(policy_name) {
        Ok(p) => p,
        Err(code) => return code,
    };

    let mut enforcer = Enforcer::new(policy);
    let decision = enforcer.authorize(&request);

    println!("Policy: {policy_name}");
    println!("Request: {resource:?} {access:?} scope={:?}", request.scope);
    if !request.context.is_empty() {
        println!("Context: {:?}", request.context);
    }
    match &decision.matched_rule {
        Some(id) => println!("Matched rule: {id}"),
        None => println!("Matched rule: none (fell through to the default effect)"),
    }
    println!("Reason: {}", decision.reason);

    if decision.allowed {
        println!("✓ ALLOWED");
        0
    } else {
        println!("✗ DENIED");
        1
    }
}

fn cmd_describe(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: aios-security describe <policy>");
        return 2;
    }

    let mut policy = match load_policy(&args[0]) {
        Ok(p) => p,
        Err(code) => return code,
    };
    // Show the order requests are actually evaluated in, not the file order.
    policy.normalize();

    print!("{}", aios_security::enforce::describe(&policy));
    0
}

fn cmd_contain_profiles() -> i32 {
    let manager = ContainManager::with_builtin_profiles();
    let names = manager.profile_names();

    println!("Available contain profiles ({}):", names.len());
    for name in &names {
        let cfg = &manager.get_profile(name).expect("listed profile exists");
        println!(
            "  {name}: rootfs={} mounts={} network={} isolation={}",
            cfg.rootfs.display(),
            cfg.mounts.len(),
            if cfg.network.enabled { "on" } else { "off" },
            if cfg.network.isolate {
                "isolated"
            } else {
                "shared"
            },
        );
    }

    println!("\nShow what a profile grants, without starting anything:");
    println!("  aios-security contain-plan <profile>");

    if !manager.is_available() {
        println!(
            "\nNote: /scheme/contain is not mounted here, so containers cannot be \
             created. Planning works regardless; see docs/gotchas/security-isolation.md."
        );
    }
    0
}

fn cmd_contain_plan(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: aios-security contain-plan <profile> [container-name]");
        eprintln!("profiles: ai-model, dev");
        return 2;
    }

    let profile = &args[0];
    let name = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("{profile}-container"));

    let manager = ContainManager::with_builtin_profiles();
    match manager.plan_container(name, profile) {
        Ok(plan) => {
            print!("{}", plan.summary());
            println!(
                "\nThis is a plan, not a running container. Creating one requires the \
                 Redox `contain` scheme."
            );
            0
        }
        Err(e) => {
            eprintln!("error planning container for profile '{profile}': {e}");
            eprintln!("available profiles: {}", manager.profile_names().join(", "));
            1
        }
    }
}
