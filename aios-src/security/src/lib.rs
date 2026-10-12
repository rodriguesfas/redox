//! AIOS Security - AI Model Isolation and Permissions
//!
//! Provides permission model and isolation using Redox's `contain` scheme.

pub mod contain;
pub mod enforce;
pub mod permissions;
pub mod policy;
pub mod registry;

pub use contain::{ContainConfig, ContainError, ContainManager, ContainerInfo, ContainerPlan};
pub use enforce::{AccessDenied, AccessRequest, Enforcer};
pub use permissions::{AIPermissions, AccessLevel, PermissionRule, PermissionSet, ResourceType};
pub use policy::{PolicyDecision, PolicyEffect, PolicyRule, SecurityPolicy};
pub use registry::{PolicyEntry, PolicyRegistry};

use std::path::PathBuf;

/// Default policy registry path
pub fn default_policy_registry_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aios")
        .join("policies.toml")
}
