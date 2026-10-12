//! Integration tests for aios-deploy

use aios_deploy::{
    validate_deployment, CompatibilityCheck, DeployConfig, DeployPlan, DeployStatus, DeployStep,
    DeviceEntry, DeviceRegistry, DeviceStatus, TransferConfig,
};
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_device_registry_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let registry_path = temp_dir.path().join("devices.toml");

    let mut reg = DeviceRegistry::load_from(&registry_path).unwrap();
    assert!(reg.is_empty());

    // Add device
    let mut entry = DeviceEntry::new(
        "test-device".to_string(),
        "192.168.1.100".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );
    entry.ram_mb = 4096;
    entry.storage_mb = 16384;

    let id = reg.add(entry);
    assert!(!id.is_empty());
    assert_eq!(reg.len(), 1);

    // Save and reload
    reg.save().unwrap();

    let reg2 = DeviceRegistry::load_from(&registry_path).unwrap();
    assert_eq!(reg2.len(), 1);

    let device = reg2.get(&id).unwrap();
    assert_eq!(device.name, "test-device");
    assert_eq!(device.address, "192.168.1.100");
    assert_eq!(device.arch, "aarch64");
    assert_eq!(device.ram_mb, 4096);
    assert_eq!(device.storage_mb, 16384);
}

#[test]
fn test_device_registry_find_by_name() {
    let temp_dir = TempDir::new().unwrap();
    let registry_path = temp_dir.path().join("devices.toml");

    let mut reg = DeviceRegistry::load_from(&registry_path).unwrap();

    let entry = DeviceEntry::new(
        "edge-01".to_string(),
        "10.0.2.15".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );
    let id = reg.add(entry);
    reg.save().unwrap();

    let found = reg.find_by_name("edge-01").unwrap();
    assert_eq!(found.id, id);

    assert!(reg.find_by_name("nonexistent").is_none());
}

#[test]
fn test_device_registry_remove() {
    let temp_dir = TempDir::new().unwrap();
    let registry_path = temp_dir.path().join("devices.toml");

    let mut reg = DeviceRegistry::load_from(&registry_path).unwrap();

    let entry = DeviceEntry::new(
        "to-remove".to_string(),
        "10.0.0.1".to_string(),
        "x86_64".to_string(),
        "redox".to_string(),
    );
    let id = reg.add(entry);
    reg.save().unwrap();

    assert_eq!(reg.len(), 1);

    let removed = reg.remove(&id).unwrap();
    assert_eq!(removed.name, "to-remove");
    assert_eq!(reg.len(), 0);

    reg.save().unwrap();
    let reg2 = DeviceRegistry::load_from(&registry_path).unwrap();
    assert!(reg2.is_empty());
}

#[test]
fn test_device_status_update() {
    let temp_dir = TempDir::new().unwrap();
    let registry_path = temp_dir.path().join("devices.toml");

    let mut reg = DeviceRegistry::load_from(&registry_path).unwrap();

    let entry = DeviceEntry::new(
        "status-test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );
    let id = reg.add(entry);
    reg.save().unwrap();

    // Update status
    reg.update_status(&id, DeviceStatus::Online);
    reg.save().unwrap();

    let reg2 = DeviceRegistry::load_from(&registry_path).unwrap();
    assert_eq!(reg2.get(&id).unwrap().status, DeviceStatus::Online);
}

#[test]
fn test_device_models_update() {
    let temp_dir = TempDir::new().unwrap();
    let registry_path = temp_dir.path().join("devices.toml");

    let mut reg = DeviceRegistry::load_from(&registry_path).unwrap();

    let entry = DeviceEntry::new(
        "model-test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );
    let id = reg.add(entry);
    reg.save().unwrap();

    // Update models
    let models = vec!["model1".to_string(), "model2".to_string()];
    reg.update_models(&id, models.clone());
    reg.save().unwrap();

    let reg2 = DeviceRegistry::load_from(&registry_path).unwrap();
    assert_eq!(reg2.get(&id).unwrap().models, models);
}

#[test]
fn test_device_can_run_model() {
    let mut entry = DeviceEntry::new(
        "test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );
    entry.ram_mb = 4096;
    entry.storage_mb = 16384;

    // Same arch, enough RAM
    assert!(entry.can_run_model(1000, "aarch64"));
    assert!(entry.can_run_model(1000, "any"));

    // Different arch
    assert!(!entry.can_run_model(1000, "x86_64"));

    // Not enough RAM
    entry.ram_mb = 512;
    assert!(!entry.can_run_model(1000, "aarch64"));
}

#[test]
fn test_deploy_plan_creation() {
    let device = DeviceEntry::new(
        "test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );

    let config = DeployConfig {
        device_id: device.id.clone(),
        model_path: PathBuf::from("/tmp/model.gguf"),
        model_name: Some("test-model".to_string()),
        force: false,
        verify_checksum: true,
        backup_existing: false,
        start_service: true,
    };

    let plan = DeployPlan::new(
        &config,
        &device,
        100 * 1024 * 1024,
        "abc123".to_string(),
        "test-model".to_string(),
    );

    assert_eq!(plan.device_id, device.id);
    assert_eq!(plan.model_name, "test-model");
    assert_eq!(plan.model_size, 100 * 1024 * 1024);
    assert_eq!(plan.model_sha256, "abc123");
    assert_eq!(plan.status, DeployStatus::Pending);
    assert!(!plan.steps.is_empty());
    assert!(plan.steps.contains(&DeployStep::ValidateCompatibility));
    assert!(plan.steps.contains(&DeployStep::TransferModel));
}

#[test]
fn test_deploy_plan_next_step() {
    let device = DeviceEntry::new(
        "test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );

    let config = DeployConfig {
        device_id: device.id.clone(),
        model_path: PathBuf::from("/tmp/model.gguf"),
        model_name: Some("test-model".to_string()),
        force: false,
        verify_checksum: true,
        backup_existing: false,
        start_service: true,
    };

    let plan = DeployPlan::new(
        &config,
        &device,
        100 * 1024 * 1024,
        "abc123".to_string(),
        "test-model".to_string(),
    );

    let completed = vec![];
    let next = plan.next_step(&completed).unwrap();
    assert_eq!(*next, DeployStep::ValidateCompatibility);

    let completed = vec![DeployStep::ValidateCompatibility];
    let next = plan.next_step(&completed).unwrap();
    assert_eq!(*next, DeployStep::CheckStorage);

    let completed = plan.steps.clone();
    assert!(plan.next_step(&completed).is_none());
    assert!(plan.is_complete(&completed));
}

#[test]
fn test_compatibility_check_defaults() {
    let check = CompatibilityCheck::new();
    assert!(!check.compatible);
    assert!(!check.arch_match);
    assert!(!check.ram_sufficient);
    assert!(!check.storage_sufficient);
    assert!(!check.model_exists);
    assert!(check.warnings.is_empty());
    assert!(check.errors.is_empty());
}

#[test]
fn test_transfer_config_defaults() {
    let config = TransferConfig::default();
    assert_eq!(config.base_url, "http://127.0.0.1:8989");
    assert_eq!(config.timeout_secs, 300);
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.chunk_size, 1024 * 1024);
    assert!(config.verify_checksum);
}

#[test]
fn test_validate_deployment_missing_model() {
    let temp_dir = TempDir::new().unwrap();
    let model_path = temp_dir.path().join("nonexistent.gguf");

    let device = DeviceEntry::new(
        "test".to_string(),
        "10.0.0.1".to_string(),
        "aarch64".to_string(),
        "redox".to_string(),
    );

    let check = validate_deployment(&device, &model_path).unwrap();
    assert!(!check.model_exists);
    assert!(!check.compatible);
    assert!(check.errors.iter().any(|e| e.contains("not found")));
}
