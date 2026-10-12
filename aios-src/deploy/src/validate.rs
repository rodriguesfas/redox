//! Validation for AIOS Edge Deployment
//!
//! Compatibility checks before deployment.

use crate::device_registry::DeviceEntry;
use aios_core::ModelMeta;
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompatibilityCheck {
    pub compatible: bool,
    pub arch_match: bool,
    pub ram_sufficient: bool,
    pub storage_sufficient: bool,
    pub model_exists: bool,
    pub required_ram_mb: u64,
    pub available_ram_mb: u64,
    pub required_storage_mb: u64,
    pub available_storage_mb: u64,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl CompatibilityCheck {
    pub fn new() -> Self {
        Self {
            compatible: false,
            arch_match: false,
            ram_sufficient: false,
            storage_sufficient: false,
            model_exists: false,
            required_ram_mb: 0,
            available_ram_mb: 0,
            required_storage_mb: 0,
            available_storage_mb: 0,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn add_warning(&mut self, w: String) {
        self.warnings.push(w);
    }

    pub fn add_error(&mut self, e: String) {
        self.errors.push(e);
    }

    pub fn finalize(&mut self) {
        self.compatible = self.arch_match
            && self.ram_sufficient
            && self.storage_sufficient
            && self.model_exists
            && self.errors.is_empty();
    }
}

impl Default for CompatibilityCheck {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a deployment before execution
pub fn validate_deployment(device: &DeviceEntry, model_path: &Path) -> Result<CompatibilityCheck> {
    let mut check = CompatibilityCheck::new();

    // Check model file exists
    if !model_path.exists() {
        check.add_error(format!("Model file not found: {}", model_path.display()));
        check.finalize();
        return Ok(check);
    }
    check.model_exists = true;

    // Parse model metadata to get size and arch
    let meta = ModelMeta::from_path(model_path).map_err(|e| {
        anyhow::anyhow!(
            "reading model metadata from {}: {}",
            model_path.display(),
            e
        )
    })?;

    let model_size_mb = meta.size_bytes / (1024 * 1024);
    check.required_storage_mb = model_size_mb * 2; // 2x for temp + installed

    // Check architecture
    let model_arch = meta
        .header
        .as_ref()
        .and_then(|h| h.metadata.iter().find(|m| m.key == "general.architecture"))
        .and_then(|m| match &m.value {
            aios_core::gguf::Value::String(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("unknown");

    check.arch_match = device.arch == model_arch || model_arch == "unknown" || model_arch == "any";
    if !check.arch_match {
        check.add_error(format!(
            "Architecture mismatch: device={} model={}",
            device.arch, model_arch
        ));
    }

    // Check RAM (model needs ~1.5x for weights + context + overhead)
    check.required_ram_mb = model_size_mb * 3 / 2 + 256; // +256MB overhead
    check.available_ram_mb = device.ram_mb;
    check.ram_sufficient = device.ram_mb >= check.required_ram_mb;
    if !check.ram_sufficient {
        check.add_error(format!(
            "Insufficient RAM: need {} MB, device has {} MB",
            check.required_ram_mb, device.ram_mb
        ));
    } else if device.ram_mb < check.required_ram_mb * 2 {
        check.add_warning(format!(
            "RAM may be tight: need {} MB, device has {} MB (recommend 2x)",
            check.required_ram_mb, device.ram_mb
        ));
    }

    // Check storage
    check.available_storage_mb = device.storage_mb;
    check.storage_sufficient = device.storage_mb >= check.required_storage_mb;
    if !check.storage_sufficient {
        check.add_error(format!(
            "Insufficient storage: need {} MB, device has {} MB",
            check.required_storage_mb, device.storage_mb
        ));
    }

    // Validate model format
    if meta.header.is_none() {
        check.add_warning("File does not appear to be a valid GGUF model".to_string());
    }

    check.finalize();
    Ok(check)
}

/// Quick check if a device can run a model by size/arch
pub fn quick_compatibility(device: &DeviceEntry, model_size_mb: u64, model_arch: &str) -> bool {
    device.can_run_model(model_size_mb, model_arch)
}
