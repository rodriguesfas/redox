//! Project scaffolding for `aios-dev new` (Fase 2).
//!
//! Two things here are not cosmetic:
//!
//! * **Dependency paths are resolved, not assumed.** A scaffolded crate refers
//!   to the in-tree AIOS crates (`aios-core`, `aios-sdk`, ...) by path, so the
//!   scaffolder locates the workspace instead of writing a relative guess that
//!   only works one directory below the repo root.
//! * **The generated crate detaches from any parent workspace.** Creating
//!   `myapp/` inside the AIOS checkout produces a package that cargo refuses to
//!   build ("current package believes it's in a workspace when it's not"),
//!   because the generated `Cargo.toml` declares no workspace of its own.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::project::{ProjectManifest, TEMPLATES};

/// Crates a generated project may depend on, keyed by the name written into
/// `Cargo.toml`. Order is stable so output is diffable.
const WORKSPACE_CRATES: &[(&str, &str)] = &[
    ("aios-core", "ai-core"),
    ("aios-sdk", "edge-sdk"),
    ("aios-deploy", "deploy"),
    ("aios-security", "security"),
];

/// Why a scaffold could not be produced.
#[derive(Debug)]
pub enum ScaffoldError {
    /// The target directory already exists.
    Exists(PathBuf),
    /// A template name that the scaffolder does not know.
    UnknownTemplate {
        name: String,
        known: &'static [&'static str],
    },
    /// No AIOS workspace could be located to point path dependencies at.
    NoWorkspace,
    /// The located workspace is missing a crate the template needs.
    MissingCrate { crate_name: String, path: PathBuf },
    /// Filesystem error while writing.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ScaffoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScaffoldError::Exists(p) => write!(f, "directory '{}' already exists", p.display()),
            ScaffoldError::UnknownTemplate { name, known } => {
                write!(f, "unknown template '{name}' (known: {})", known.join(", "))
            }
            ScaffoldError::NoWorkspace => write!(
                f,
                "could not locate the AIOS workspace (expected a directory containing \
                 ai-core/Cargo.toml and a root Cargo.toml with [workspace]).\n\
                 Set AIOS_WORKSPACE=/path/to/aios and retry."
            ),
            ScaffoldError::MissingCrate { crate_name, path } => write!(
                f,
                "workspace crate '{crate_name}' not found at {}",
                path.display()
            ),
            ScaffoldError::Io { path, source } => {
                write!(f, "writing {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ScaffoldError {}

/// Locate the AIOS workspace root.
///
/// Order: explicit `AIOS_WORKSPACE`, then the ancestors of the current
/// directory, then the ancestors of the running executable (so the CLI works
/// when invoked from anywhere, including `target/debug/dev`).
pub fn resolve_workspace() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("AIOS_WORKSPACE") {
        let p = PathBuf::from(explicit);
        if is_workspace(&p) {
            return Some(p.canonicalize().unwrap_or(p));
        }
        return None;
    }

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = find_workspace_upwards(&cwd) {
            return Some(found);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(found) = find_workspace_upwards(dir) {
                return Some(found);
            }
        }
    }

    None
}

/// A directory is the workspace root when it holds `ai-core/Cargo.toml` and a
/// `Cargo.toml` that declares `[workspace]`.
fn is_workspace(dir: &Path) -> bool {
    if !dir.join("ai-core").join("Cargo.toml").is_file() {
        return false;
    }
    match std::fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(text) => text.contains("[workspace]"),
        Err(_) => false,
    }
}

fn find_workspace_upwards(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| is_workspace(dir))
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

/// Express `target` relative to `from_dir`.
///
/// Returns an absolute path instead when the two only share a filesystem root
/// (`/tmp/x` vs `/home/y`), where a relative path would be a long chain of
/// `..` that is correct but unreadable and easy to break by moving the
/// project one directory.
fn relativize(from_dir: &Path, target: &Path) -> PathBuf {
    let from = from_dir
        .canonicalize()
        .unwrap_or_else(|_| from_dir.to_path_buf());
    let to = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());

    let from_parts: Vec<_> = from.components().collect();
    let to_parts: Vec<_> = to.components().collect();

    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Index 0 is the root/prefix; sharing only that means unrelated trees.
    if common < 2 {
        return to;
    }

    let mut out = PathBuf::new();
    for component in &from_parts[common..] {
        if matches!(component, Component::Normal(_)) {
            out.push("..");
        }
    }
    for component in &to_parts[common..] {
        out.push(component.as_os_str());
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Result of a successful scaffold.
#[derive(Debug)]
pub struct Scaffolded {
    pub project_dir: PathBuf,
    pub manifest: ProjectManifest,
    pub files: Vec<PathBuf>,
}

/// Scaffold `name` from `template` into `parent`.
///
/// The `aios-core` / `aios-sdk` path dependencies are written relative to the
/// new project, so the project builds both inside and outside the checkout.
pub fn scaffold(parent: &Path, name: &str, template: &str) -> Result<Scaffolded, ScaffoldError> {
    if !TEMPLATES.contains(&template) {
        return Err(ScaffoldError::UnknownTemplate {
            name: template.to_string(),
            known: TEMPLATES,
        });
    }

    let project_dir = parent.join(name);
    if project_dir.exists() {
        return Err(ScaffoldError::Exists(project_dir));
    }

    let workspace = resolve_workspace().ok_or(ScaffoldError::NoWorkspace)?;

    // Resolve the crates we are about to reference, and fail loudly rather
    // than emitting a Cargo.toml that cannot resolve later.
    let mut crate_paths: Vec<(String, PathBuf)> = Vec::new();
    for (dep_name, dir_name) in WORKSPACE_CRATES {
        let abs = workspace.join(dir_name);
        if !abs.join("Cargo.toml").is_file() {
            return Err(ScaffoldError::MissingCrate {
                crate_name: (*dep_name).to_string(),
                path: abs,
            });
        }
        crate_paths.push(((*dep_name).to_string(), abs));
    }

    let manifest = build_manifest(name, template, &project_dir, &crate_paths);

    let cargo_toml = render_cargo_toml(name, &crate_paths, &project_dir);
    let main_rs = render_main_rs(name, template);

    std::fs::create_dir_all(project_dir.join("src")).map_err(|source| ScaffoldError::Io {
        path: project_dir.clone(),
        source,
    })?;

    let manifest_toml = manifest.to_toml().map_err(|e| ScaffoldError::Io {
        path: project_dir.join("project.toml"),
        source: std::io::Error::other(e.to_string()),
    })?;

    let mut files = Vec::new();
    for (rel, contents) in [
        ("Cargo.toml", cargo_toml),
        ("project.toml", manifest_toml),
        ("src/main.rs", main_rs),
    ] {
        let path = project_dir.join(rel);
        std::fs::write(&path, contents).map_err(|source| ScaffoldError::Io {
            path: path.clone(),
            source,
        })?;
        files.push(path);
    }

    Ok(Scaffolded {
        project_dir,
        manifest,
        files,
    })
}

fn build_manifest(
    name: &str,
    template: &str,
    project_dir: &Path,
    crate_paths: &[(String, PathBuf)],
) -> ProjectManifest {
    use crate::project::{ModelSpec, ProjectMeta, ProjectPermissions, TaskSpec, DEFAULT_TARGET};
    use std::collections::BTreeMap;

    let mut meta = ProjectMeta {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        template: Some(template.to_string()),
        target: Some(DEFAULT_TARGET.to_string()),
    };

    let mut model = None;
    let mut permissions = None;
    let mut task = None;

    match template {
        "edge-service" => {
            model = Some(ModelSpec {
                name: "tinyllama.q4_k_m".to_string(),
                format: Some("gguf".to_string()),
                required_ram_mb: Some(1200),
            });
            // A service is the one template that needs isolation by default:
            // it binds a port and reads the model cache.
            permissions = Some(ProjectPermissions {
                filesystem_read: vec!["/var/lib/ai/models".to_string()],
                network_inbound: vec![8989],
                network_outbound: vec!["127.0.0.1".to_string()],
                max_memory_mb: Some(2048),
                isolate: true,
                contain_profile: Some("ai-model".to_string()),
                ..Default::default()
            });
        }
        "agent" => {
            task = Some(TaskSpec {
                name: "sentiment".to_string(),
                max_tokens: Some(64),
            });
            model = Some(ModelSpec {
                name: "tinyllama.q4_k_m".to_string(),
                format: Some("gguf".to_string()),
                required_ram_mb: Some(1200),
            });
        }
        "classifier" => {
            task = Some(TaskSpec {
                name: "classify".to_string(),
                max_tokens: Some(32),
            });
            model = Some(ModelSpec {
                name: "tinyllama.q4_k_m".to_string(),
                format: Some("gguf".to_string()),
                required_ram_mb: Some(1200),
            });
        }
        // `minimal` stays cross-platform: a `target` here would break
        // `cargo build` on any non-Redox host for no benefit.
        _ => meta.target = None,
    }

    let dependencies: BTreeMap<String, crate::project::DependencySpec> = crate_paths
        .iter()
        .take(2) // minimal set: aios-core + aios-sdk
        .map(|(dep, abs)| {
            let rel = relativize(project_dir, abs);
            (
                dep.clone(),
                crate::project::DependencySpec::Path {
                    path: rel.to_string_lossy().into_owned(),
                },
            )
        })
        .collect();

    ProjectManifest {
        project: meta,
        model,
        permissions,
        task,
        dependencies,
    }
}

fn render_cargo_toml(name: &str, crate_paths: &[(String, PathBuf)], project_dir: &Path) -> String {
    let mut out = String::new();
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{name}\"\n"));
    out.push_str("version = \"0.1.0\"\n");
    out.push_str("edition = \"2021\"\n\n");
    // Detach from any enclosing workspace: without this, `cargo build` inside
    // the AIOS checkout fails with "current package believes it's in a
    // workspace when it's not".
    out.push_str("# Standalone crate: do not absorb into the AIOS workspace.\n");
    out.push_str("[workspace]\n\n");
    out.push_str("[dependencies]\n");
    for (dep, abs) in crate_paths {
        let rel = relativize(project_dir, abs);
        out.push_str(&format!(
            "{dep} = {{ path = \"{}\" }}\n",
            rel.to_string_lossy()
        ));
    }
    out
}

fn render_main_rs(name: &str, template: &str) -> String {
    let header = format!("//! {name} — AIOS {template} template\n");
    let body = match template {
        "edge-service" => {
            r#"use aios_sdk::ai;

fn main() {
    println!("{name}: edge service starting");
    match ai::load("tinyllama.q4_k_m") {
        Ok(model) => println!("loaded model: {}", model.name()),
        Err(e) => eprintln!("could not load model: {e}"),
    }
}
"#
        }
        "agent" => {
            r#"use aios_sdk::ai;

fn main() {
    println!("{name}: agent ready");
    match ai::load("tinyllama.q4_k_m") {
        Ok(model) => println!("loaded model: {}", model.name()),
        Err(e) => eprintln!("could not load model: {e}"),
    }
}
"#
        }
        "classifier" => {
            r#"use aios_sdk::ai;

fn main() {
    println!("{name}: classifier ready");
    match ai::load("tinyllama.q4_k_m") {
        Ok(model) => println!("loaded model: {}", model.name()),
        Err(e) => eprintln!("could not load model: {e}"),
    }
}
"#
        }
        _ => {
            r#"fn main() {
    println!("hello from {name}");
}
"#
        }
    };
    // `{name}` is substituted after formatting so the template bodies stay
    // readable as plain Rust.
    format!("{header}\n{}", body.replace("{name}", name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// `scaffold` resolves the workspace through the process-wide
    /// `AIOS_WORKSPACE`, so every test here is affected by every other one.
    /// `fails_clearly_when_the_workspace_is_absent` in particular points that
    /// variable at a directory which is deliberately NOT a workspace, so any
    /// test that overlaps it gets `NoWorkspace` instead of the workspace it
    /// built. Serializing them removes the race for the cost of a mutex.
    ///
    /// This is a real flake: it surfaced in CI as
    /// `generated_cargo_toml_declares_its_own_workspace` failing while every
    /// test in the crate passed on its own.
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let m = LOCK.get_or_init(|| Mutex::new(()));
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A throwaway AIOS-shaped workspace: the scaffolder only needs the crate
    /// directories and the root manifest to exist.
    fn fake_workspace(root: &Path) {
        std::fs::create_dir_all(root.join("ai-core")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        std::fs::write(
            root.join("ai-core/Cargo.toml"),
            "[package]\nname = \"aios-core\"\n",
        )
        .unwrap();
        for dir in ["edge-sdk", "deploy", "security"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(
                root.join(dir).join("Cargo.toml"),
                format!("[package]\nname = \"aios-{dir}\"\n"),
            )
            .unwrap();
        }
    }

    #[test]
    fn scaffolds_every_template_and_produces_a_valid_manifest() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        for template in TEMPLATES {
            let out = scaffold(tmp.path(), template, template).unwrap();
            assert_eq!(out.manifest.project.template.as_deref(), Some(*template));
            assert!(
                out.manifest.validate().is_empty(),
                "template {template} produced invalid manifest: {:?}",
                out.manifest.validate()
            );
            assert!(out.project_dir.join("Cargo.toml").is_file());
            assert!(out.project_dir.join("project.toml").is_file());
            assert!(out.project_dir.join("src/main.rs").is_file());
        }
    }

    #[test]
    fn generated_manifest_parses_from_disk_and_round_trips() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        let out = scaffold(tmp.path(), "svc", "edge-service").unwrap();
        let loaded = ProjectManifest::load(out.project_dir.join("project.toml")).unwrap();
        assert_eq!(loaded.project.name, "svc");
        assert_eq!(loaded.model_name(), "tinyllama.q4_k_m");
    }

    #[test]
    fn path_dependencies_resolve_from_the_generated_project() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        // Scaffold deep and off to the side: the old hardcoded "../../ai-core"
        // would not resolve from here.
        let nested = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        let out = scaffold(&nested, "myapp", "minimal").unwrap();

        let manifest = ProjectManifest::load(out.project_dir.join("project.toml")).unwrap();
        for spec in manifest.dependencies.values() {
            if let crate::project::DependencySpec::Path { path } = spec {
                let resolved = out.project_dir.join(path).join("Cargo.toml");
                assert!(
                    resolved.is_file(),
                    "dependency path '{path}' does not resolve to a crate (looked at {})",
                    resolved.display()
                );
            }
        }
    }

    #[test]
    fn generated_cargo_toml_declares_its_own_workspace() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        let out = scaffold(tmp.path(), "myapp", "minimal").unwrap();
        let cargo_toml = std::fs::read_to_string(out.project_dir.join("Cargo.toml")).unwrap();
        assert!(
            cargo_toml.contains("[workspace]"),
            "generated Cargo.toml must detach from any parent workspace:\n{cargo_toml}"
        );
    }

    #[test]
    fn edge_service_template_requests_isolation() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        let out = scaffold(tmp.path(), "svc", "edge-service").unwrap();
        let perms = out.manifest.permissions.as_ref().unwrap();
        assert!(perms.isolate);
        assert_eq!(perms.network_inbound, vec![8989]);
        // And the projection into the security engine preserves it.
        let ai_perms = perms.to_ai_permissions("tinyllama");
        assert!(ai_perms.isolate);
    }

    #[test]
    fn minimal_template_omits_a_target_so_it_builds_on_any_host() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        let out = scaffold(tmp.path(), "myapp", "minimal").unwrap();
        assert!(out.manifest.project.target.is_none());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_directory() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        scaffold(tmp.path(), "myapp", "minimal").unwrap();
        let err = scaffold(tmp.path(), "myapp", "minimal").unwrap_err();
        assert!(matches!(err, ScaffoldError::Exists(_)));
    }

    #[test]
    fn rejects_an_unknown_template_by_name() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let err = scaffold(tmp.path(), "myapp", "does-not-exist").unwrap_err();
        match err {
            ScaffoldError::UnknownTemplate { name, known } => {
                assert_eq!(name, "does-not-exist");
                assert!(known.contains(&"edge-service"));
            }
            other => panic!("expected UnknownTemplate, got {other:?}"),
        }
    }

    #[test]
    fn fails_clearly_when_the_workspace_is_absent() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        // Point AIOS_WORKSPACE at a directory that is not a workspace.
        std::env::set_var("AIOS_WORKSPACE", tmp.path());
        let result = scaffold(outside.path(), "myapp", "minimal");
        std::env::remove_var("AIOS_WORKSPACE");
        assert!(matches!(result, Err(ScaffoldError::NoWorkspace)));
    }

    #[test]
    fn relativize_walks_up_and_down() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("a/b");
        let to = tmp.path().join("a/c/d");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        assert_eq!(relativize(&from, &to), PathBuf::from("../c/d"));
    }

    #[test]
    fn relativize_keeps_paths_absolute_across_unrelated_trees() {
        // /tmp and /home share only the root: a `../../../..` chain would
        // resolve, but is unreadable and fragile. Prefer absolute.
        let rel = relativize(
            Path::new("/tmp/proj/myapp"),
            Path::new("/home/user/aios/ai-core"),
        );
        assert_eq!(rel, PathBuf::from("/home/user/aios/ai-core"));
        assert!(
            !rel.to_string_lossy().contains(".."),
            "unrelated trees must not produce a .. chain: {}",
            rel.display()
        );
    }

    #[test]
    fn scaffold_outside_the_checkout_uses_absolute_dependency_paths() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fake_workspace(&ws);

        // A "workspace" in one tree, the project in another: the generated
        // manifest must still point at resolvable, readable paths.
        let elsewhere = tempfile::tempdir().unwrap();
        let out = scaffold(elsewhere.path(), "myapp", "minimal").unwrap();

        let manifest = ProjectManifest::load(out.project_dir.join("project.toml")).unwrap();
        let mut checked = 0;
        for spec in manifest.dependencies.values() {
            if let crate::project::DependencySpec::Path { path } = spec {
                assert!(
                    out.project_dir.join(path).join("Cargo.toml").is_file(),
                    "dependency '{path}' must resolve from the generated project"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "expected path dependencies in the manifest");
    }
}
