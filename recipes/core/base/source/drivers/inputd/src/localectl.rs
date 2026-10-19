//! `localectl`: the keyboard mapping of the console.
//!
//! `set-keymap` writes the mapping to `/etc/vconsole.conf` and asks the running
//! `inputd` to use it now; `apply` only does the second half, and is what the
//! boot service runs — `inputd` starts from the initfs, before `/etc` exists,
//! so the file is read by `apply` once the root filesystem is mounted.
//!
//! A system that is not running `inputd` (a rescue shell) still reads and
//! writes the file: the mapping is applied at the next boot.

use std::path::Path;
use std::process::ExitCode;

use inputd::keymap::KeymapKind;
use inputd::vconsole;

const USAGE: &str = "\
localectl: the keyboard mapping of the console

localectl [status]             show the mapping of /etc/vconsole.conf
localectl set-keymap <name>    write it and switch to it now
localectl apply                switch to the mapping of the file, if there is one
localectl list-keymaps         list the mappings
localectl --help               this help
";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    let result = match args.next().as_deref() {
        None | Some("status") => status(),
        Some("set-keymap") => match args.next() {
            Some(name) => set_keymap(&name),
            None => Err("set-keymap needs a keymap (see: localectl list-keymaps)".to_owned()),
        },
        Some("apply") => apply(),
        Some("list-keymaps") => list_keymaps(),
        Some("-h") | Some("--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command: {other}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("localectl: {err}");
            eprintln!("Try 'localectl --help' for more information.");
            ExitCode::FAILURE
        }
    }
}

/// The mapping the user named: `br-abnt2`, `abnt2`, `us`, ...
fn keymap_of(name: &str) -> Result<KeymapKind, String> {
    name.to_ascii_lowercase()
        .parse::<KeymapKind>()
        .map_err(|_| format!("unknown keymap: {name} (see: localectl list-keymaps)"))
}

/// Asks the running `inputd` to use a mapping.
fn activate(kind: KeymapKind) -> std::io::Result<()> {
    inputd::ControlHandle::new()?
        .activate_keymap(kind as usize)
        .map(|_| ())
}

fn status() -> Result<(), String> {
    match vconsole::load(Path::new(vconsole::PATH)) {
        Some(kind) => println!("System keymap: {kind}"),
        None => println!("System keymap: {} (default)", KeymapKind::US),
    }

    Ok(())
}

fn set_keymap(name: &str) -> Result<(), String> {
    let kind = keymap_of(name)?;
    let path = Path::new(vconsole::PATH);

    vconsole::save(path, kind)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    println!("System keymap set to {kind} in {}", path.display());

    match activate(kind) {
        Ok(()) => println!("Keymap applied to inputd"),
        Err(err) => {
            println!("inputd is not running ({err}); the keymap applies at the next boot")
        }
    }

    Ok(())
}

/// Applies the mapping of the file: what the boot service runs.
fn apply() -> Result<(), String> {
    let Some(kind) = vconsole::load(Path::new(vconsole::PATH)) else {
        // A console without a mapping keeps the default layout.
        return Ok(());
    };

    activate(kind).map_err(|err| format!("failed to apply {kind}: {err}"))?;
    println!("Keymap applied to inputd: {kind}");

    Ok(())
}

fn list_keymaps() -> Result<(), String> {
    for kind in KeymapKind::ALL {
        println!("{kind}");
    }

    Ok(())
}
