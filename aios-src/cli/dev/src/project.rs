//! `project.toml` manifest: the AIOS project descriptor (Fase 2).
//!
//! Every project scaffolded by `aios-dev new` carries a `project.toml`
//! describing the project, the model it uses, its permissions and its task.
//! The manifest is the hand-off point between the developer tooling, the AI
//! runtime (`ai` CLI) and the security engine (`aios-security`), so it
//! validates strictly instead of being decorative.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use aios_security::permissions::PermissionRule;
use aios_security::{AIPermissions, AccessLevel, ResourceType};

/// Templates `aios-dev new` knows how to scaffold.
pub const TEMPLATES: &[&str] = &["minimal", "edge-service", "agent", "classifier"];

/// Default cross-compilation target for generated projects.
pub const DEFAULT_TARGET: &str = "x86_64-unknown-redox";

/// A complete `project.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub project: ProjectMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<ProjectPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskSpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

/// `[project]` table: identity and build target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// `[model]` table: the SLM the project runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_ram_mb: Option<u64>,
}

/// `[task]` table: the default AITask the project performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// `[permissions]` table, in the readable shape project authors write.
///
/// Kept distinct from [`aios_security::AIPermissions`] on purpose: this is
/// authored by hand, while the security engine's form is generated. See
/// [`ProjectPermissions::to_ai_permissions`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPermissions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filesystem_read: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filesystem_write: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_outbound: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_inbound: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u8>,
    /// Run the model under `contain` isolation.
    #[serde(default)]
    pub isolate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contain_profile: Option<String>,
}

impl ProjectPermissions {
    /// Project these permissions into the security engine's model so a
    /// `project.toml` and an `aios-security` policy are the same thing.
    pub fn to_ai_permissions(&self, model_name: &str) -> AIPermissions {
        let mut perms = AIPermissions::new(model_name);
        for path in &self.filesystem_read {
            perms.permissions.add_rule(
                PermissionRule::new(ResourceType::Filesystem, AccessLevel::Read)
                    .with_scope(path.clone()),
            );
        }
        for path in &self.filesystem_write {
            perms.permissions.add_rule(
                PermissionRule::new(ResourceType::Filesystem, AccessLevel::Write)
                    .with_scope(path.clone()),
            );
        }
        for host in &self.network_outbound {
            perms.permissions.add_rule(
                PermissionRule::new(ResourceType::Network, AccessLevel::Write)
                    .with_scope(host.clone()),
            );
        }
        for port in &self.network_inbound {
            perms.permissions.allow_network_inbound(*port);
        }
        for device in &self.device {
            perms.permissions.allow_device(device.clone());
        }
        if self.max_memory_mb.is_some() || self.max_cpu_percent.is_some() {
            perms
                .permissions
                .allow_compute(self.max_memory_mb, self.max_cpu_percent);
        }
        if self.isolate {
            perms = perms.with_isolation(
                self.contain_profile
                    .clone()
                    .unwrap_or_else(|| "ai-model".to_string()),
            );
        }
        perms
    }
}

/// A dependency entry. Only the shapes the scaffolder emits are accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// `{ path = "..." }` — the in-tree AIOS crates.
    Path { path: String },
    /// A bare version string, e.g. `"1.0"`.
    Version(String),
}

/// A single manifest problem. `check` reports these; they never abort parsing,
/// so one run can surface every issue at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl ProjectManifest {
    /// Read and parse a `project.toml`, surfacing parse errors as validation
    /// errors so callers have a single error channel.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Vec<ValidationError>> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| {
            vec![ValidationError {
                field: path.display().to_string(),
                message: format!("cannot read manifest: {e}"),
            }]
        })?;
        Self::parse(&text)
    }

    /// Parse manifest TOML without touching the filesystem.
    pub fn parse(text: &str) -> Result<Self, Vec<ValidationError>> {
        toml::from_str(text).map_err(|e| {
            vec![ValidationError {
                field: "<manifest>".to_string(),
                message: format!("invalid TOML: {e}"),
            }]
        })
    }

    /// Serialize back to TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Report every problem found. An empty result means the manifest is
    /// usable as-is.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        let name = self.project.name.trim();
        if name.is_empty() {
            errors.push(err("project.name", "must not be empty"));
        } else if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            errors.push(err(
                "project.name",
                &format!("'{name}' is not a valid crate name (letters, digits, '-', '_' only)"),
            ));
        }

        if !is_semver(&self.project.version) {
            errors.push(err(
                "project.version",
                &format!("'{}' is not MAJOR.MINOR.PATCH", self.project.version),
            ));
        }

        if let Some(template) = &self.project.template {
            if !TEMPLATES.contains(&template.as_str()) {
                errors.push(err(
                    "project.template",
                    &format!(
                        "unknown template '{template}' (known: {})",
                        TEMPLATES.join(", ")
                    ),
                ));
            }
        }

        if let Some(model) = &self.model {
            if model.name.trim().is_empty() {
                errors.push(err("model.name", "must not be empty"));
            }
            if let Some(format) = &model.format {
                if !format.eq_ignore_ascii_case("gguf") {
                    errors.push(err(
                        "model.format",
                        &format!("unsupported format '{format}' (only gguf)"),
                    ));
                }
            }
            if model.required_ram_mb == Some(0) {
                errors.push(err("model.required_ram_mb", "must be greater than 0"));
            }
        }

        if let Some(task) = &self.task {
            if task.name.trim().is_empty() {
                errors.push(err("task.name", "must not be empty"));
            }
            if task.max_tokens == Some(0) {
                errors.push(err("task.max_tokens", "must be greater than 0"));
            }
        }

        if let Some(perms) = &self.permissions {
            if perms.isolate && perms.contain_profile.is_none() && perms.max_memory_mb.is_none() {
                // An isolation request with neither profile nor limit gives the
                // security engine nothing to enforce.
                errors.push(err(
                    "permissions",
                    "isolate = true needs contain_profile or max_memory_mb to be enforceable",
                ));
            }
            for path in perms.filesystem_read.iter().chain(&perms.filesystem_write) {
                if !path.starts_with('/') {
                    errors.push(err(
                        "permissions.filesystem_*",
                        &format!("'{path}' must be an absolute path"),
                    ));
                }
            }
            if perms.max_cpu_percent.is_some_and(|c| c == 0 || c > 100) {
                errors.push(err(
                    "permissions.max_cpu_percent",
                    "must be between 1 and 100",
                ));
            }
        }

        errors
    }

    /// Convenience: load, parse and validate in one call.
    pub fn load_validated(path: impl AsRef<Path>) -> Result<Self, Vec<ValidationError>> {
        let manifest = Self::load(path)?;
        let errors = manifest.validate();
        if errors.is_empty() {
            Ok(manifest)
        } else {
            Err(errors)
        }
    }

    /// The model name, from `[model]` or the project name as a fallback.
    pub fn model_name(&self) -> &str {
        self.model
            .as_ref()
            .map(|m| m.name.as_str())
            .unwrap_or(&self.project.name)
    }

    /// Effective build target.
    pub fn target(&self) -> &str {
        self.project.target.as_deref().unwrap_or(DEFAULT_TARGET)
    }
}

fn err(field: &str, message: &str) -> ValidationError {
    ValidationError {
        field: field.to_string(),
        message: message.to_string(),
    }
}

/// `MAJOR.MINOR.PATCH` with no pre-release/build metadata: the scaffolder only
/// ever emits this shape, so anything richer is a mistake worth reporting.
fn is_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[project]
name = "my-app"
version = "0.1.0"
template = "edge-service"
target = "x86_64-unknown-redox"

[model]
name = "tinyllama.q4_k_m"
required_ram_mb = 1200
format = "gguf"

[task]
name = "summarize"
max_tokens = 64

[dependencies]
aios-core = { path = "../../ai-core" }
serde = "1.0"
"#;

    #[test]
    fn parses_and_validates_a_full_manifest() {
        let m = ProjectManifest::parse(VALID).expect("valid manifest");
        assert!(m.validate().is_empty(), "unexpected: {:?}", m.validate());
        assert_eq!(m.project.name, "my-app");
        assert_eq!(m.model_name(), "tinyllama.q4_k_m");
        assert_eq!(m.target(), "x86_64-unknown-redox");
        assert_eq!(m.task.unwrap().max_tokens, Some(64));
    }

    #[test]
    fn target_falls_back_to_the_default() {
        let m = ProjectManifest::parse("[project]\nname = \"a\"\nversion = \"0.1.0\"\n").unwrap();
        assert_eq!(m.target(), DEFAULT_TARGET);
        assert_eq!(m.model_name(), "a");
    }

    #[test]
    fn rejects_invalid_project_name() {
        let m =
            ProjectManifest::parse("[project]\nname = \"my app!\"\nversion = \"0.1.0\"\n").unwrap();
        let errors = m.validate();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "project.name");
    }

    #[test]
    fn rejects_non_semver_version() {
        for bad in ["0.1", "0.1.0-rc1", "v0.1.0", "0.1.0.0", ""] {
            let text = format!("[project]\nname = \"a\"\nversion = \"{bad}\"\n");
            let m = ProjectManifest::parse(&text).unwrap();
            let errors = m.validate();
            assert!(
                errors.iter().any(|e| e.field == "project.version"),
                "version '{bad}' should be rejected"
            );
        }
    }

    #[test]
    fn rejects_unknown_template() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\ntemplate = \"nope\"\n",
        )
        .unwrap();
        assert!(m.validate().iter().any(|e| e.field == "project.template"));
    }

    #[test]
    fn rejects_zero_max_tokens() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[task]\nname = \"t\"\nmax_tokens = 0\n",
        )
        .unwrap();
        assert!(m.validate().iter().any(|e| e.field == "task.max_tokens"));
    }

    #[test]
    fn rejects_relative_permission_paths() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[permissions]\nfilesystem_read = [\"models\"]\n",
        )
        .unwrap();
        assert!(m
            .validate()
            .iter()
            .any(|e| e.field == "permissions.filesystem_*"));
    }

    #[test]
    fn rejects_isolate_without_anything_enforceable() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[permissions]\nisolate = true\n",
        )
        .unwrap();
        assert!(m.validate().iter().any(|e| e.field == "permissions"));
    }

    #[test]
    fn accept_isolate_with_contain_profile() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[permissions]\nisolate = true\ncontain_profile = \"ai-model\"\n",
        )
        .unwrap();
        assert!(m.validate().is_empty(), "{:?}", m.validate());
    }

    #[test]
    fn rejects_out_of_range_cpu_percent() {
        for bad in ["0", "101"] {
            let text = format!(
                "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[permissions]\nmax_cpu_percent = {bad}\n"
            );
            let m = ProjectManifest::parse(&text).unwrap();
            assert!(m
                .validate()
                .iter()
                .any(|e| e.field == "permissions.max_cpu_percent"));
        }
    }

    #[test]
    fn rejects_unsupported_model_format() {
        let m = ProjectManifest::parse(
            "[project]\nname = \"a\"\nversion = \"0.1.0\"\n\n[model]\nname = \"m\"\nformat = \"onnx\"\n",
        )
        .unwrap();
        assert!(m.validate().iter().any(|e| e.field == "model.format"));
    }

    #[test]
    fn reports_toml_syntax_error() {
        let errors = ProjectManifest::parse("[project\nname = ").unwrap_err();
        assert_eq!(errors[0].field, "<manifest>");
        assert!(errors[0].message.contains("invalid TOML"));
    }

    #[test]
    fn permissions_project_into_the_security_engine() {
        let m = ProjectManifest::parse(
            r#"
[project]
name = "svc"
version = "0.1.0"

[permissions]
filesystem_read = ["/var/lib/ai/models"]
network_inbound = [8989]
network_outbound = ["127.0.0.1"]
max_memory_mb = 2048
isolate = true
contain_profile = "ai-model"
"#,
        )
        .unwrap();
        assert!(m.validate().is_empty(), "{:?}", m.validate());

        let perms = m
            .permissions
            .as_ref()
            .unwrap()
            .to_ai_permissions("tinyllama");
        assert_eq!(perms.model_name, "tinyllama");
        assert!(perms.isolate);
        assert_eq!(perms.contain_profile.as_deref(), Some("ai-model"));

        // The projected rules must actually answer the engine's own questions.
        assert!(perms.permissions.check(
            aios_security::ResourceType::Filesystem,
            AccessLevel::Read,
            Some("/var/lib/ai/models/tinyllama.gguf"),
        ));
        assert!(!perms.permissions.check(
            aios_security::ResourceType::Filesystem,
            AccessLevel::Read,
            Some("/etc/shadow"),
        ));
        assert!(perms.permissions.check(
            aios_security::ResourceType::Network,
            AccessLevel::Read,
            None
        ));
    }

    #[test]
    fn round_trips_through_toml() {
        let m = ProjectManifest::parse(VALID).unwrap();
        let text = m.to_toml().expect("serialize");
        let again = ProjectManifest::parse(&text).expect("reparse");
        assert_eq!(m.project.name, again.project.name);
        assert_eq!(m.target(), again.target());
    }

    #[test]
    fn load_reports_a_missing_file_as_a_validation_error() {
        let errors = ProjectManifest::load("/nonexistent/project.toml").unwrap_err();
        assert!(errors[0].message.contains("cannot read manifest"));
    }
}
