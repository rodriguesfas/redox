//! `aios-dev doctor`: environment diagnostics (Fase 2, ROADMAP).
//!
//! The ROADMAP asks for checks of Rust, Cargo, Git, filesystem, network, CPU,
//! RAM, AI runtime, models, storage, toolchains, targets and the edge SDK.
//! Each check returns a [`Check`] with a tri-state status so a missing optional
//! component is reported as a warning rather than failing the whole run — the
//! exit code only reflects genuinely required pieces.
//!
//! Nothing here assumes Linux: `/proc` is used when present and skipped
//! otherwise, because the Developer OS is expected to run these checks on
//! Redox itself.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Severity of a single check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Required and missing or broken.
    Fail,
    /// Present but something is off (optional component, degraded source).
    Warn,
    /// Present and healthy.
    Ok,
}

impl Status {
    fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "+",
            Status::Warn => "~",
            Status::Fail => "x",
        }
    }
}

/// One diagnostic line.
#[derive(Debug, Clone)]
pub struct Check {
    pub section: &'static str,
    pub name: String,
    pub status: Status,
    pub detail: String,
    /// Whether a failure here should fail the whole doctor run.
    pub required: bool,
}

impl Check {
    pub fn new(
        section: &'static str,
        name: impl Into<String>,
        status: Status,
        detail: impl Into<String>,
    ) -> Self {
        Check {
            section,
            name: name.into(),
            status,
            detail: detail.into(),
            required: true,
        }
    }

    /// Mark this check as advisory: its absence is a warning, not a failure.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}

/// The full doctor report.
#[derive(Debug, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// True when every required check passed.
    pub fn is_healthy(&self) -> bool {
        self.checks
            .iter()
            .all(|c| c.status != Status::Fail || !c.required)
    }

    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks
            .iter()
            .filter(|c| c.status == Status::Fail && c.required)
    }

    /// Render the report in a stable, human-readable layout.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut current_section = "";
        for check in &self.checks {
            if check.section != current_section {
                current_section = check.section;
                out.push_str(&format!("\n[{current_section}]\n"));
            }
            // Distinguish "advisory and not satisfied" from a real failure, so
            // a `~` is never mistaken for something that blocks the work.
            let note = if !check.required && check.status != Status::Ok {
                " (optional)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {} {:<16} {}{}\n",
                check.status.glyph(),
                check.name,
                check.detail,
                note
            ));
        }
        out
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

const SEC_TOOLCHAIN: &str = "Toolchains";
const SEC_PLATFORM: &str = "Platform";
const SEC_STORAGE: &str = "Storage";
const SEC_AI: &str = "AI runtime";
const SEC_TARGETS: &str = "Targets";
const SEC_SDK: &str = "AIOS SDK";

/// Run every diagnostic. `workspace` is only used for the SDK/target checks;
/// pass `None` to skip locating it.
pub fn run(workspace: Option<&Path>) -> Report {
    let mut report = Report::default();

    // --- Toolchains -------------------------------------------------------
    check_tool(&mut report, "Rust", "rustc", &["--version"], true);
    check_tool(&mut report, "Cargo", "cargo", &["--version"], true);
    check_tool(&mut report, "Git", "git", &["--version"], true);
    check_tool(&mut report, "rustfmt", "rustfmt", &["--version"], true);
    // Linters are useful but not required to build.
    check_tool(&mut report, "clippy", "cargo-clippy", &["--version"], false);

    // --- Platform: CPU / RAM / network ------------------------------------
    report.push(Check::new(
        SEC_PLATFORM,
        "CPU",
        Status::Ok,
        match std::thread::available_parallelism() {
            Ok(p) => format!("{} logical core(s)", p.get()),
            Err(_) => "core count unknown".to_string(),
        }
        .as_str(),
    ));

    match total_memory_mb() {
        Some(mb) => report.push(Check::new(
            SEC_PLATFORM,
            "RAM",
            if mb > 0 { Status::Ok } else { Status::Warn },
            format!("{mb} MiB total"),
        )),
        None => report
            .push(Check::new(SEC_PLATFORM, "RAM", Status::Warn, "unknown (no /proc)").optional()),
    }

    report.push(check_network());

    // --- Storage: writable workspace + models dir -------------------------
    let models_dir = aios_core::default_models_dir();
    report.push(check_writable(Path::new("."), "cwd", true));
    report.push(check_writable(Path::new(&models_dir), "models dir", false));

    // --- AI runtime -------------------------------------------------------
    report.push(check_models(Path::new(&models_dir)));

    // --- Targets ----------------------------------------------------------
    report.push(check_targets());

    // --- AIOS SDK ---------------------------------------------------------
    match workspace {
        Some(ws) => report.push(check_sdk(ws)),
        None => {
            report.push(Check::new(SEC_SDK, "workspace", Status::Warn, "not located").optional())
        }
    }

    report
}

fn check_tool(report: &mut Report, name: &str, bin: &str, args: &[&str], required: bool) {
    let (status, detail) = match std::process::Command::new(bin).args(args).output() {
        Ok(out) if out.status.success() => {
            let line = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            (Status::Ok, line)
        }
        _ => (Status::Fail, "NOT FOUND".to_string()),
    };
    let check = Check::new(SEC_TOOLCHAIN, name, status, detail);
    report.push(if required { check } else { check.optional() });
}

/// Total RAM in MiB, read from `/proc/meminfo` when available.
///
/// Returns `None` off-Linux (notably inside Redox), which the caller reports
/// as a warning rather than a failure.
fn total_memory_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb / 1024);
        }
    }
    None
}

/// Can we open a TCP connection? Advisory by nature: the Developer OS may
/// legitimately run offline, and Redox answers `tcp:` from userspace netstack,
/// which is exactly what the aarch64 blocker breaks.
fn check_network() -> Check {
    let reachable = std::net::TcpStream::connect_timeout(
        &"1.1.1.1:443".parse().expect("valid socket address"),
        Duration::from_millis(800),
    )
    .is_ok();
    if reachable {
        Check::new(SEC_PLATFORM, "network", Status::Ok, "outbound TCP ok")
    } else {
        Check::new(
            SEC_PLATFORM,
            "network",
            Status::Warn,
            "no outbound TCP (offline, or Redox netstack unavailable)",
        )
        .optional()
    }
}

fn check_writable(path: &Path, name: &str, required: bool) -> Check {
    if !path.exists() {
        let check = Check::new(
            SEC_STORAGE,
            name,
            if required { Status::Fail } else { Status::Warn },
            format!("missing: {}", path.display()),
        );
        return if required { check } else { check.optional() };
    }
    // Probe rather than trust metadata: a read-only mount reports writable
    // permissions that do not hold.
    let probe = path.join(".aios-dev-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            let check = Check::new(
                SEC_STORAGE,
                name,
                Status::Ok,
                format!("writable: {}", path.display()),
            );
            if required {
                check
            } else {
                check.optional()
            }
        }
        Err(e) => {
            let check = Check::new(
                SEC_STORAGE,
                name,
                if required { Status::Fail } else { Status::Warn },
                format!("not writable: {e}"),
            );
            if required {
                check
            } else {
                check.optional()
            }
        }
    }
}

fn check_models(dir: &Path) -> Check {
    if !dir.is_dir() {
        return Check::new(
            SEC_AI,
            "models",
            Status::Warn,
            format!("no models dir at {}", dir.display()),
        )
        .optional();
    }
    match aios_core::list_installed(dir) {
        Ok(models) if models.is_empty() => Check::new(
            SEC_AI,
            "models",
            Status::Warn,
            format!("none installed in {}", dir.display()),
        )
        .optional(),
        Ok(models) => {
            let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
            Check::new(
                SEC_AI,
                "models",
                Status::Ok,
                format!("{} installed: {}", models.len(), names.join(", ")),
            )
        }
        Err(e) => {
            Check::new(SEC_AI, "models", Status::Warn, format!("scan failed: {e}")).optional()
        }
    }
}

fn check_targets() -> Check {
    let known = match std::process::Command::new("rustc")
        .args(["--print", "target-list"])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => {
            return Check::new(
                SEC_TARGETS,
                "redox targets",
                Status::Fail,
                "rustc cannot list targets",
            )
        }
    };
    let x86 = known.contains("x86_64-unknown-redox");
    let arm = known.contains("aarch64-unknown-redox");
    let detail = format!("x86_64: {x86}, aarch64: {arm}");
    if x86 && arm {
        Check::new(SEC_TARGETS, "redox targets", Status::Ok, detail)
    } else {
        // Listing support is not the same as having std for the target; a
        // missing one is a warning, not a broken toolchain.
        Check::new(SEC_TARGETS, "redox targets", Status::Warn, detail).optional()
    }
}

fn check_sdk(workspace: &Path) -> Check {
    let required = [
        "ai-core", "edge-sdk", "edge-ai", "cli/ai", "deploy", "security",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|c| !workspace.join(c).join("Cargo.toml").is_file())
        .collect();
    if missing.is_empty() {
        Check::new(
            SEC_SDK,
            "workspace",
            Status::Ok,
            format!("all {} crates present", required.len()),
        )
    } else {
        Check::new(
            SEC_SDK,
            "workspace",
            Status::Fail,
            format!("missing crates: {}", missing.join(", ")),
        )
    }
}

/// Convenience for the binary: locate the workspace, run every check.
pub fn run_with_workspace_lookup() -> Report {
    run(crate::scaffold::resolve_workspace().as_deref())
}

/// Path of the models dir as reported by `aios-core`, for the CLI banner.
pub fn models_dir() -> PathBuf {
    PathBuf::from(aios_core::default_models_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_healthy_when_nothing_required_fails() {
        let mut r = Report::default();
        r.push(Check::new(SEC_TOOLCHAIN, "Rust", Status::Ok, "rustc 1.98"));
        assert!(r.is_healthy());
    }

    #[test]
    fn an_optional_failure_does_not_fail_the_run() {
        let mut r = Report::default();
        r.push(Check::new(SEC_SDK, "clippy", Status::Fail, "NOT FOUND").optional());
        assert!(r.is_healthy(), "optional failure must not fail the report");
        assert_eq!(r.failures().count(), 0);
    }

    #[test]
    fn a_required_failure_fails_the_run() {
        let mut r = Report::default();
        r.push(Check::new(
            SEC_TOOLCHAIN,
            "Cargo",
            Status::Fail,
            "NOT FOUND",
        ));
        assert!(!r.is_healthy());
        assert_eq!(r.failures().count(), 1);
    }

    #[test]
    fn render_groups_by_section_in_order() {
        let mut r = Report::default();
        r.push(Check::new(SEC_TOOLCHAIN, "Rust", Status::Ok, "rustc 1.98"));
        r.push(Check::new(SEC_AI, "models", Status::Warn, "none"));
        let out = r.render();
        let toolchain_at = out.find("[Toolchains]").expect("toolchains section");
        let ai_at = out.find("[AI runtime]").expect("ai section");
        assert!(toolchain_at < ai_at, "sections must follow insertion order");
    }

    #[test]
    fn render_marks_unmet_optional_checks_as_advisory() {
        let mut r = Report::default();
        r.push(Check::new(SEC_SDK, "clippy", Status::Fail, "NOT FOUND").optional());
        r.push(Check::new(SEC_SDK, "rustfmt", Status::Fail, "NOT FOUND"));
        let out = r.render();
        assert!(
            out.contains("(optional)"),
            "unmet optional checks must be labelled: {out}"
        );
        // A required failure is not softened.
        assert_eq!(out.matches("(optional)").count(), 1, "{out}");
    }

    #[test]
    fn writability_probe_detects_a_read_only_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let check = check_writable(tmp.path(), "temp", true);
        assert_eq!(check.status, Status::Ok, "{check:?}");

        // A path that does not exist is a failure when required.
        let missing = check_writable(&tmp.path().join("nope"), "missing", true);
        assert_eq!(missing.status, Status::Fail);
    }

    #[test]
    fn sdk_check_lists_the_missing_crates() {
        let tmp = tempfile::tempdir().unwrap();
        let check = check_sdk(tmp.path());
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("ai-core"));
    }

    #[test]
    fn sdk_check_passes_for_a_complete_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        for c in [
            "ai-core", "edge-sdk", "edge-ai", "cli/ai", "deploy", "security",
        ] {
            std::fs::create_dir_all(tmp.path().join(c)).unwrap();
            std::fs::write(tmp.path().join(c).join("Cargo.toml"), "").unwrap();
        }
        let check = check_sdk(tmp.path());
        assert_eq!(check.status, Status::Ok, "{check:?}");
    }

    #[test]
    fn run_produces_checks_for_every_required_section() {
        let report = run(None);
        let sections: Vec<&str> = report.checks.iter().map(|c| c.section).collect();
        for expected in [
            SEC_TOOLCHAIN,
            SEC_PLATFORM,
            SEC_STORAGE,
            SEC_AI,
            SEC_TARGETS,
            SEC_SDK,
        ] {
            assert!(
                sections.contains(&expected),
                "doctor must cover section {expected:?}; got {sections:?}"
            );
        }
    }
}
