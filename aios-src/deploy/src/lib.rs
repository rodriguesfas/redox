//! AIOS Edge Deployment
//!
//! Device registry and model deployment protocol for Fase 7.

pub mod deploy_protocol;
pub mod device_registry;
pub mod transfer;
pub mod validate;

pub use deploy_protocol::{DeployConfig, DeployPlan, DeployStatus, DeployStep};
pub use device_registry::{DeviceEntry, DeviceRegistry, DeviceStatus};
pub use transfer::{health_check, transfer_model, HealthReport, TransferConfig, TransferResult};
pub use validate::{validate_deployment, CompatibilityCheck};

use std::path::PathBuf;

/// Default path for device registry
pub fn default_device_registry_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aios")
        .join("devices.toml")
}

/// Default deploy state directory
pub fn default_deploy_state_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aios")
        .join("deploy")
}
