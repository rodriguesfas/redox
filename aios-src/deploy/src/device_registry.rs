//! Device Registry for AIOS Edge Deployment
//!
//! Manages the registry of edge devices (name, address, arch, OS, RAM, storage, models, status, last_seen).

use crate::default_device_registry_path;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
    pub address: String,
    pub arch: String,
    pub os: String,
    pub ram_mb: u64,
    pub storage_mb: u64,
    pub models: Vec<String>,
    pub status: DeviceStatus,
    pub last_seen: u64,
    pub added_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    /// The default: we registered the device but have not heard from it yet.
    /// Defaulting to `Online` would report a device as reachable purely because
    /// it is in a file.
    #[default]
    Unknown,
    Online,
    Offline,
    Deploying,
    Error(String),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceRegistryFile {
    devices: HashMap<String, DeviceEntry>,
}

pub struct DeviceRegistry {
    path: std::path::PathBuf,
    devices: HashMap<String, DeviceEntry>,
}

impl DeviceRegistry {
    /// Load registry from default path
    pub fn load() -> Result<Self> {
        let path = default_device_registry_path();
        Self::load_from(&path)
    }

    /// Load registry from specific path
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let devices = if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("reading device registry from {}", path.display()))?;
            let file: DeviceRegistryFile = toml::from_str(&content)
                .with_context(|| format!("parsing device registry from {}", path.display()))?;
            file.devices
        } else {
            HashMap::new()
        };
        Ok(Self { path, devices })
    }

    /// Save registry to disk
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        let file = DeviceRegistryFile {
            devices: self.devices.clone(),
        };
        let content = toml::to_string_pretty(&file).context("serializing device registry")?;
        fs::write(&self.path, content)
            .with_context(|| format!("writing device registry to {}", self.path.display()))?;
        Ok(())
    }

    /// Add or update a device
    pub fn upsert(&mut self, mut entry: DeviceEntry) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        entry.last_seen = now;
        if entry.id.is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        if entry.added_at == 0 {
            entry.added_at = now;
        }
        self.devices.insert(entry.id.clone(), entry);
    }

    /// Add a device with auto-generated ID
    pub fn add(&mut self, mut entry: DeviceEntry) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if entry.id.is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        entry.added_at = now;
        entry.last_seen = now;
        let id = entry.id.clone();
        self.devices.insert(id.clone(), entry);
        id
    }

    /// Remove a device by ID
    pub fn remove(&mut self, id: &str) -> Option<DeviceEntry> {
        self.devices.remove(id)
    }

    /// Find device by ID
    pub fn get(&self, id: &str) -> Option<&DeviceEntry> {
        self.devices.get(id)
    }

    /// Find device by name
    pub fn find_by_name(&self, name: &str) -> Option<&DeviceEntry> {
        self.devices.values().find(|d| d.name == name)
    }

    /// List all devices
    pub fn list(&self) -> Vec<&DeviceEntry> {
        self.devices.values().collect()
    }

    /// Update device status
    pub fn update_status(&mut self, id: &str, status: DeviceStatus) -> bool {
        if let Some(entry) = self.devices.get_mut(id) {
            entry.status = status;
            entry.last_seen = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            true
        } else {
            false
        }
    }

    /// Update device models list
    pub fn update_models(&mut self, id: &str, models: Vec<String>) -> bool {
        if let Some(entry) = self.devices.get_mut(id) {
            entry.models = models;
            entry.last_seen = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            true
        } else {
            false
        }
    }

    /// Update device last_seen timestamp
    pub fn touch(&mut self, id: &str) -> bool {
        if let Some(entry) = self.devices.get_mut(id) {
            entry.last_seen = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            true
        } else {
            false
        }
    }

    /// Get device count
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

impl DeviceEntry {
    /// Create a new device entry with minimal info
    pub fn new(name: String, address: String, arch: String, os: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            address,
            arch,
            os,
            ram_mb: 0,
            storage_mb: 0,
            models: Vec::new(),
            status: DeviceStatus::Unknown,
            last_seen: now,
            added_at: now,
        }
    }

    /// Check if device is compatible with a model
    pub fn can_run_model(&self, model_size_mb: u64, model_arch: &str) -> bool {
        if self.arch != model_arch && model_arch != "any" {
            return false;
        }
        // Rough estimate: model needs ~1.5x RAM for weights + context
        let required_ram = model_size_mb * 3 / 2;
        self.ram_mb >= required_ram
    }

    /// Get device summary for display
    pub fn summary(&self) -> String {
        format!(
            "{} ({}) @ {}  {}MB RAM  {}MB storage  {}  models={}  last_seen={}",
            self.name,
            &self.id[..8.min(self.id.len())],
            self.address,
            self.ram_mb,
            self.storage_mb,
            self.status_str(),
            self.models.len(),
            self.last_seen
        )
    }

    pub fn status_str(&self) -> &str {
        match self.status {
            DeviceStatus::Online => "online",
            DeviceStatus::Offline => "offline",
            DeviceStatus::Deploying => "deploying",
            DeviceStatus::Error(_) => "error",
            DeviceStatus::Unknown => "unknown",
        }
    }
}
