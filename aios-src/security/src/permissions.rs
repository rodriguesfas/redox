//! AI Permissions Model
//!
//! Defines the permission types for AI model isolation.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Resource types that can be accessed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    /// Filesystem access (read/write)
    Filesystem,
    /// Network access (inbound/outbound)
    Network,
    /// Device access (GPU, NPU, cameras, sensors)
    Device,
    /// Compute resources (CPU, memory limits)
    Compute,
    /// Inter-process communication
    Ipc,
    /// System information (hardware, OS version)
    SystemInfo,
}

/// Access level for a resource
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    /// No access. The default, so an unset grant is never accidentally a grant.
    #[default]
    None,
    /// Read-only access
    Read,
    /// Write-only access
    Write,
    /// Full read/write access
    ReadWrite,
    /// Execute access (for compute/device)
    Execute,
}

/// A single permission rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub resource: ResourceType,
    pub access: AccessLevel,
    /// Optional path/scope restriction (e.g., "/var/lib/ai/models" for filesystem)
    pub scope: Option<String>,
    /// Optional conditions (e.g., "model:llama", "port:8989")
    pub conditions: HashSet<String>,
}

impl PermissionRule {
    pub fn new(resource: ResourceType, access: AccessLevel) -> Self {
        Self {
            resource,
            access,
            scope: None,
            conditions: HashSet::new(),
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.conditions.insert(condition.into());
        self
    }

    /// Check if this rule allows the given access
    pub fn allows(&self, resource: ResourceType, access: AccessLevel, scope: Option<&str>) -> bool {
        if self.resource != resource {
            return false;
        }
        // Check access level
        let access_ok = match (self.access, access) {
            (AccessLevel::None, _) => false,
            (AccessLevel::Read, AccessLevel::Read) => true,
            (AccessLevel::Write, AccessLevel::Write) => true,
            (AccessLevel::ReadWrite, AccessLevel::Read) => true,
            (AccessLevel::ReadWrite, AccessLevel::Write) => true,
            (AccessLevel::ReadWrite, AccessLevel::ReadWrite) => true,
            (AccessLevel::Execute, AccessLevel::Execute) => true,
            _ => false,
        };
        if !access_ok {
            return false;
        }

        // Scope check. No scope on the rule matches any request scope; a scoped
        // rule requires a request scope inside it. Delegated to
        // `enforce::scope_contains` so the policy layer and this runtime path
        // cannot drift apart — a `starts_with` here would reintroduce the
        // shared-prefix and `..`-traversal holes that the policy layer fixed.
        match self.scope.as_deref() {
            None => true,
            Some(rule_scope) => match scope {
                None => false,
                Some(request_scope) => {
                    crate::enforce::scope_contains(self.resource, rule_scope, request_scope)
                }
            },
        }
    }
}

/// Complete permission set for an AI model/service
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionSet {
    pub rules: Vec<PermissionRule>,
}

impl PermissionSet {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    pub fn allow_filesystem_read(&mut self, path: impl Into<String>) {
        self.add_rule(
            PermissionRule::new(ResourceType::Filesystem, AccessLevel::Read).with_scope(path),
        );
    }

    pub fn allow_filesystem_write(&mut self, path: impl Into<String>) {
        self.add_rule(
            PermissionRule::new(ResourceType::Filesystem, AccessLevel::Write).with_scope(path),
        );
    }

    pub fn allow_network_outbound(&mut self, host: Option<impl Into<String>>) {
        let mut rule = PermissionRule::new(ResourceType::Network, AccessLevel::Write);
        if let Some(h) = host {
            rule = rule.with_scope(h);
        }
        self.add_rule(rule);
    }

    pub fn allow_network_inbound(&mut self, port: u16) {
        self.add_rule(
            PermissionRule::new(ResourceType::Network, AccessLevel::Read)
                .with_condition(format!("port:{}", port)),
        );
    }

    pub fn allow_device(&mut self, device_type: impl Into<String>) {
        self.add_rule(
            PermissionRule::new(ResourceType::Device, AccessLevel::Execute)
                .with_condition(device_type),
        );
    }

    pub fn allow_compute(&mut self, max_memory_mb: Option<u64>, max_cpu_percent: Option<u8>) {
        let mut rule = PermissionRule::new(ResourceType::Compute, AccessLevel::Execute);
        if let Some(m) = max_memory_mb {
            rule = rule.with_condition(format!("max_memory:{}", m));
        }
        if let Some(c) = max_cpu_percent {
            rule = rule.with_condition(format!("max_cpu:{}", c));
        }
        self.add_rule(rule);
    }

    /// Check if permission is granted
    pub fn check(&self, resource: ResourceType, access: AccessLevel, scope: Option<&str>) -> bool {
        self.rules.iter().any(|r| r.allows(resource, access, scope))
    }

    /// Get all rules for a resource type
    pub fn rules_for(&self, resource: ResourceType) -> Vec<&PermissionRule> {
        self.rules
            .iter()
            .filter(|r| r.resource == resource)
            .collect()
    }
}

/// High-level AI permissions configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIPermissions {
    pub model_name: String,
    pub permissions: PermissionSet,
    /// Run inside contain isolation
    pub isolate: bool,
    /// Contain profile name
    pub contain_profile: Option<String>,
}

impl AIPermissions {
    pub fn new(model_name: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            permissions: PermissionSet::new(),
            isolate: false,
            contain_profile: None,
        }
    }

    pub fn with_isolation(mut self, profile: impl Into<String>) -> Self {
        self.isolate = true;
        self.contain_profile = Some(profile.into());
        self
    }

    pub fn permissive(model_name: impl Into<String>) -> Self {
        let mut perms = Self::new(model_name);
        perms.permissions.add_rule(PermissionRule::new(
            ResourceType::Filesystem,
            AccessLevel::ReadWrite,
        ));
        perms.permissions.add_rule(PermissionRule::new(
            ResourceType::Network,
            AccessLevel::ReadWrite,
        ));
        perms.permissions.add_rule(PermissionRule::new(
            ResourceType::Device,
            AccessLevel::Execute,
        ));
        perms.permissions.add_rule(PermissionRule::new(
            ResourceType::Compute,
            AccessLevel::Execute,
        ));
        perms
    }

    pub fn restricted(model_name: impl Into<String>) -> Self {
        let mut perms = Self::new(model_name);
        perms
            .permissions
            .allow_filesystem_read("/var/lib/ai/models");
        perms
            .permissions
            .allow_filesystem_write("/tmp/ai-inference");
        perms.permissions.allow_network_inbound(8989);
        perms.permissions.allow_compute(Some(2048), Some(80));
        perms
    }

    pub fn edge_default(model_name: impl Into<String>) -> Self {
        let mut perms = Self::restricted(model_name);
        perms.permissions.allow_network_outbound(Some("127.0.0.1"));
        perms.isolate = true;
        perms.contain_profile = Some("ai-model".to_string());
        perms
    }
}
