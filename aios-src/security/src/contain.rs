//! Redox Contain Integration
//!
//! Manages isolation containers using Redox's `contain` scheme.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContainError {
    #[error("Contain scheme not available: {0}")]
    SchemeUnavailable(String),
    #[error("Container creation failed: {0}")]
    CreationFailed(String),
    #[error("Container not found: {0}")]
    NotFound(String),
    #[error("Operation not permitted: {0}")]
    NotPermitted(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Contain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainConfig {
    pub profile: String,
    pub rootfs: PathBuf,
    pub mounts: Vec<MountConfig>,
    pub network: NetworkConfig,
    pub resources: ResourceLimits,
    pub env: HashMap<String, String>,
}

impl Default for ContainConfig {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            rootfs: PathBuf::from("/"),
            mounts: Vec::new(),
            network: NetworkConfig::default(),
            resources: ResourceLimits::default(),
            env: HashMap::new(),
        }
    }
}

/// Mount configuration for contain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub source: PathBuf,
    pub target: PathBuf,
    pub readonly: bool,
    pub options: Vec<String>,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub enabled: bool,
    pub allow_inbound: Vec<u16>,
    pub allow_outbound: Vec<String>, // CIDR or hostnames
    pub isolate: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound: Vec::new(),
            allow_outbound: Vec::new(),
            isolate: false,
        }
    }
}

/// Resource limits. Every field defaults to `None`, which means unlimited —
/// spelled out as a derive so the default cannot drift from the field list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_memory_mb: Option<u64>,
    pub max_cpu_percent: Option<u8>,
    pub max_pids: Option<u32>,
    pub max_open_files: Option<u64>,
}

/// Container status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerStatus {
    Created,
    Running,
    Stopped,
    Failed(String),
}

/// Container info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub status: ContainerStatus,
    pub pid: Option<u32>,
    pub created_at: u64,
    pub config: ContainConfig,
}

/// Contain manager for creating and managing isolation containers
pub struct ContainManager {
    contain_path: PathBuf,
    profiles: HashMap<String, ContainConfig>,
}

impl ContainManager {
    /// Create a new contain manager
    pub fn new() -> Result<Self> {
        // Check if contain scheme is available
        let contain_path = PathBuf::from("/scheme/contain");
        if !contain_path.exists() {
            return Err(ContainError::SchemeUnavailable(
                "Contain scheme not mounted at /scheme/contain".to_string(),
            )
            .into());
        }

        Ok(Self {
            contain_path,
            profiles: HashMap::new(),
        })
    }

    /// Register a contain profile
    pub fn register_profile(&mut self, name: impl Into<String>, config: ContainConfig) {
        self.profiles.insert(name.into(), config);
    }

    /// Names of the registered profiles, sorted.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// The profiles the platform ships with, as data.
    ///
    /// Kept separate from [`ContainManager::new`] because that constructor
    /// requires a mounted `contain` scheme. Planning a profile is useful off
    /// Redox — it is how you inspect what isolation a policy claims — so the
    /// definitions must be reachable without the scheme.
    pub fn builtin_profiles() -> Vec<(String, ContainConfig)> {
        let ai_model = ContainConfig {
            profile: "ai-model".to_string(),
            // Read-only model root; nothing the model can reach is writable.
            rootfs: PathBuf::from("/var/lib/ai/models"),
            mounts: vec![
                MountConfig {
                    source: PathBuf::from("/var/lib/ai/models"),
                    target: PathBuf::from("/models"),
                    readonly: true,
                    options: vec!["ro".to_string(), "nosuid".to_string()],
                },
                MountConfig {
                    source: PathBuf::from("/tmp/ai-inference"),
                    target: PathBuf::from("/tmp"),
                    readonly: false,
                    options: vec!["nodev".to_string(), "nosuid".to_string()],
                },
            ],
            network: NetworkConfig {
                enabled: true,
                // Loopback only, pinned to the local inference server.
                allow_inbound: vec![8989],
                allow_outbound: vec!["127.0.0.1".to_string()],
                // No route to anything else, so a policy bug cannot egress.
                isolate: true,
            },
            resources: ResourceLimits {
                max_memory_mb: Some(2048),
                max_cpu_percent: Some(80),
                max_pids: Some(64),
                max_open_files: Some(256),
            },
            env: HashMap::from([
                ("HOME".to_string(), "/tmp".to_string()),
                ("HF_HUB_OFFLINE".to_string(), "1".to_string()),
            ]),
        };

        let dev = ContainConfig {
            profile: "dev".to_string(),
            rootfs: PathBuf::from("/"),
            mounts: vec![MountConfig {
                source: PathBuf::from("/home"),
                target: PathBuf::from("/home"),
                readonly: false,
                options: vec!["rw".to_string()],
            }],
            network: NetworkConfig {
                enabled: true,
                allow_inbound: vec![],
                allow_outbound: vec![],
                isolate: false,
            },
            resources: ResourceLimits::default(),
            env: HashMap::new(),
        };

        vec![("ai-model".to_string(), ai_model), ("dev".to_string(), dev)]
    }

    /// A manager preloaded with [`ContainManager::builtin_profiles`].
    ///
    /// Unlike [`ContainManager::new`] this does not require `/scheme/contain`,
    /// so `contain-plan` works on a dev host. It is not an assertion that
    /// containers can be started here; use `is_available` for that.
    pub fn with_builtin_profiles() -> Self {
        let mut manager = Self {
            contain_path: PathBuf::from("/scheme/contain"),
            profiles: HashMap::new(),
        };
        for (name, config) in Self::builtin_profiles() {
            manager.profiles.insert(name, config);
        }
        manager
    }

    /// Get a profile by name
    pub fn get_profile(&self, name: &str) -> Option<&ContainConfig> {
        self.profiles.get(name)
    }

    /// Compute the isolation a container would get, without starting anything.
    ///
    /// This is the part of containment that is real and testable off-Redox: the
    /// set of mounts, network grants and limits a profile resolves to. Callers
    /// can log it, diff it against a previous run, or assert on it in tests.
    pub fn plan_container(
        &self,
        name: impl Into<String>,
        profile_name: &str,
    ) -> Result<ContainerPlan> {
        let profile = self.profiles.get(profile_name).ok_or_else(|| {
            ContainError::CreationFailed(format!("Profile '{profile_name}' not found"))
        })?;
        let name = name.into();
        let mounts: Vec<PlannedMount> = profile
            .mounts
            .iter()
            .map(|m| PlannedMount {
                source: m.source.clone(),
                target: m.target.clone(),
                readonly: m.readonly,
                options: m.options.clone(),
            })
            .collect();
        Ok(ContainerPlan {
            name,
            profile: profile_name.to_string(),
            rootfs: profile.rootfs.clone(),
            mounts,
            network: profile.network.clone(),
            resources: profile.resources.clone(),
            env: profile.env.clone(),
        })
    }

    /// Create a container from a profile.
    ///
    /// This does **not** spawn anything: the `contain` scheme is not wired up
    /// (see `docs/gotchas/security-isolation.md`). It returns
    /// [`ContainError::NotImplemented`] rather than a fabricated
    /// [`ContainerInfo`], because a caller that believes a container is
    /// running when nothing was started is worse off than one that got an
    /// error. Use [`ContainManager::plan_container`] to inspect what isolation
    /// *would* be applied.
    pub fn create_container(
        &self,
        _name: impl Into<String>,
        profile_name: &str,
    ) -> Result<ContainerInfo> {
        if !self.profiles.contains_key(profile_name) {
            return Err(ContainError::CreationFailed(format!(
                "Profile '{profile_name}' not found"
            ))
            .into());
        }
        Err(ContainError::NotImplemented(format!(
            "cannot create container for profile '{profile_name}': the Redox \
             `contain` scheme is not implemented (plan_container works)"
        ))
        .into())
    }

    /// Start a container
    pub fn start_container(&self, _container_id: &str) -> Result<ContainerInfo> {
        Err(ContainError::NotImplemented(
            "start_container: the `contain` scheme is not implemented".to_string(),
        )
        .into())
    }

    /// Stop a container
    pub fn stop_container(&self, _container_id: &str) -> Result<()> {
        Err(ContainError::NotImplemented(
            "stop_container: no containers can be running, because \
             create_container does not create any"
                .to_string(),
        )
        .into())
    }

    /// List containers. Always empty: nothing is ever created.
    pub fn list_containers(&self) -> Vec<ContainerInfo> {
        Vec::new()
    }

    /// Get container info. Always `None`: nothing is ever created.
    pub fn get_container(&self, _container_id: &str) -> Option<ContainerInfo> {
        None
    }

    /// Check if contain is available on this system.
    ///
    /// On Redox this reports whether the scheme is mounted. Off Redox it is
    /// always false, which is why the enforcement tests exercise
    /// [`crate::enforce`] instead.
    pub fn is_available(&self) -> bool {
        self.contain_path.exists()
    }

    /// Construct a manager bound to an explicit scheme path.
    ///
    /// Exists so tests can point at a temporary directory and exercise profile
    /// handling without depending on a live Redox system.
    pub fn with_scheme_path(contain_path: PathBuf) -> Self {
        Self {
            contain_path,
            profiles: HashMap::new(),
        }
    }
}

/// The isolation a container would receive, derived from its profile.
///
/// Produced by [`ContainManager::plan_container`]. It is a description, not a
/// running container: nothing has been started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPlan {
    pub name: String,
    pub profile: String,
    pub rootfs: PathBuf,
    pub mounts: Vec<PlannedMount>,
    pub network: NetworkConfig,
    pub resources: ResourceLimits,
    pub env: HashMap<String, String>,
}

/// A mount a container would receive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedMount {
    pub source: PathBuf,
    pub target: PathBuf,
    pub readonly: bool,
    pub options: Vec<String>,
}

impl ContainerPlan {
    /// A human-readable summary, for `aios-security plan`.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "container '{}' (profile '{}')\n  rootfs: {}\n",
            self.name,
            self.profile,
            self.rootfs.display()
        );
        for mount in &self.mounts {
            out.push_str(&format!(
                "  mount: {} -> {}{}\n",
                mount.source.display(),
                mount.target.display(),
                if mount.readonly { " (ro)" } else { " (rw)" }
            ));
        }
        if self.network.enabled {
            out.push_str(&format!(
                "  network: enabled, inbound {:?}, outbound {:?}\n",
                self.network.allow_inbound, self.network.allow_outbound
            ));
        } else {
            out.push_str("  network: disabled\n");
        }
        if let Some(mem) = self.resources.max_memory_mb {
            out.push_str(&format!("  max memory: {mem} MiB\n"));
        }
        if let Some(cpu) = self.resources.max_cpu_percent {
            out.push_str(&format!("  max cpu: {cpu}%\n"));
        }
        out
    }
}
