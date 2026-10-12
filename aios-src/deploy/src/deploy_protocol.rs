//! Deploy Protocol for AIOS Edge Deployment
//!
//! Defines the deployment configuration, plan, and steps for transferring
//! and installing models on edge devices.

use crate::device_registry::DeviceEntry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployConfig {
    pub device_id: String,
    pub model_path: PathBuf,
    pub model_name: Option<String>,
    pub force: bool,
    pub verify_checksum: bool,
    pub backup_existing: bool,
    pub start_service: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployPlan {
    pub id: String,
    pub device_id: String,
    pub model_path: PathBuf,
    pub model_name: String,
    pub model_size: u64,
    pub model_sha256: String,
    pub steps: Vec<DeployStep>,
    pub created_at: u64,
    pub status: DeployStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeployStatus {
    Pending,
    InProgress,
    Completed,
    Failed(String),
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStep {
    ValidateCompatibility,
    CheckStorage,
    TransferModel,
    VerifyChecksum,
    InstallModel,
    RegisterModel,
    ConfigureService,
    HealthCheck,
    StartService,
}

impl DeployPlan {
    /// Create a new deploy plan
    pub fn new(
        config: &DeployConfig,
        device: &DeviceEntry,
        model_size: u64,
        model_sha256: String,
        model_name: String,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let steps = vec![
            DeployStep::ValidateCompatibility,
            DeployStep::CheckStorage,
            DeployStep::TransferModel,
            DeployStep::VerifyChecksum,
            DeployStep::InstallModel,
            DeployStep::RegisterModel,
            if config.start_service {
                DeployStep::ConfigureService
            } else {
                DeployStep::HealthCheck
            },
        ];

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            device_id: device.id.clone(),
            model_path: config.model_path.clone(),
            model_name,
            model_size,
            model_sha256,
            steps,
            created_at: now,
            status: DeployStatus::Pending,
        }
    }

    /// Get next pending step
    pub fn next_step(&self, completed_steps: &[DeployStep]) -> Option<&DeployStep> {
        self.steps.iter().find(|s| !completed_steps.contains(s))
    }

    /// Check if all steps completed
    pub fn is_complete(&self, completed_steps: &[DeployStep]) -> bool {
        self.steps.iter().all(|s| completed_steps.contains(s))
    }
}

/// Summary of a deployment for reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploySummary {
    pub plan_id: String,
    pub device_id: String,
    pub model_name: String,
    pub status: DeployStatus,
    pub completed_steps: Vec<DeployStep>,
    pub failed_step: Option<DeployStep>,
    pub error: Option<String>,
    pub started_at: u64,
    pub completed_at: Option<u64>,
}
