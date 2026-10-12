//! Policy Registry
//!
//! Persistent storage for security policies and AI permissions.

use crate::permissions::{AIPermissions, AccessLevel, ResourceType};
use crate::policy::{PolicyEffect, PolicyRule, SecurityPolicy};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
struct PolicyRegistryFile {
    policies: HashMap<String, PolicyEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEntry {
    pub id: String,
    pub name: String,
    pub policy: SecurityPolicy,
    pub ai_permissions: Option<AIPermissions>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct PolicyRegistry {
    path: std::path::PathBuf,
    policies: HashMap<String, PolicyEntry>,
}

impl PolicyRegistry {
    /// Load registry from default path
    pub fn load() -> Result<Self> {
        let path = crate::default_policy_registry_path();
        Self::load_from(&path)
    }

    /// Load registry from specific path
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let policies = if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("reading policy registry from {}", path.display()))?;
            let file: PolicyRegistryFile = toml::from_str(&content)
                .with_context(|| format!("parsing policy registry from {}", path.display()))?;
            file.policies
        } else {
            HashMap::new()
        };
        Ok(Self { path, policies })
    }

    /// Save registry to disk
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        let file = PolicyRegistryFile {
            policies: self.policies.clone(),
        };
        let content = toml::to_string_pretty(&file).context("serializing policy registry")?;
        fs::write(&self.path, content)
            .with_context(|| format!("writing policy registry to {}", self.path.display()))?;
        Ok(())
    }

    /// Add or update a policy
    pub fn upsert(
        &mut self,
        name: impl Into<String>,
        policy: SecurityPolicy,
        ai_permissions: Option<AIPermissions>,
    ) -> String {
        let name = name.into();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let id = if let Some(existing) = self.policies.get(&name) {
            existing.id.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        let entry = PolicyEntry {
            id: id.clone(),
            name: name.clone(),
            policy,
            ai_permissions,
            created_at: self
                .policies
                .get(&name)
                .map(|e| e.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        self.policies.insert(name, entry);
        id
    }

    /// Get a policy by name
    pub fn get(&self, name: &str) -> Option<&PolicyEntry> {
        self.policies.get(name)
    }

    /// Get a policy by ID
    pub fn get_by_id(&self, id: &str) -> Option<&PolicyEntry> {
        self.policies.values().find(|e| e.id == id)
    }

    /// Remove a policy by name
    pub fn remove(&mut self, name: &str) -> Option<PolicyEntry> {
        self.policies.remove(name)
    }

    /// List all policies
    pub fn list(&self) -> Vec<&PolicyEntry> {
        self.policies.values().collect()
    }

    /// Get policy count
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Create a default restrictive policy for AI models
    pub fn create_ai_model_policy(&mut self, model_name: &str) -> String {
        let mut policy = SecurityPolicy::new(format!("ai-model-{}", model_name));
        policy.default_effect = PolicyEffect::Deny;

        // Allow reading model files
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Filesystem,
                AccessLevel::Read,
            )
            .with_scope("/var/lib/ai/models")
            .with_priority(10),
        );

        // Allow writing to temp inference directory
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Filesystem,
                AccessLevel::Write,
            )
            .with_scope("/tmp/ai-inference")
            .with_priority(10),
        );

        // Allow inbound on inference port
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Network,
                AccessLevel::Read,
            )
            .with_condition("port".to_string(), "8989".to_string())
            .with_priority(10),
        );

        // Allow compute with limits
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Compute,
                AccessLevel::Execute,
            )
            .with_condition("max_memory".to_string(), "2048".to_string())
            .with_condition("max_cpu".to_string(), "80".to_string())
            .with_priority(10),
        );

        // Deny everything else explicitly (though default is deny)
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Deny,
                ResourceType::Device,
                AccessLevel::Execute,
            )
            .with_priority(1),
        );
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Deny,
                ResourceType::Network,
                AccessLevel::Write,
            )
            .with_priority(1),
        );

        let ai_perms = AIPermissions::edge_default(model_name);

        self.upsert(model_name, policy, Some(ai_perms))
    }

    /// Create a permissive policy for development
    pub fn create_dev_policy(&mut self, name: &str) -> String {
        let mut policy = SecurityPolicy::new(format!("dev-{}", name));
        policy.default_effect = PolicyEffect::Allow;

        // Allow all resources
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Filesystem,
                AccessLevel::ReadWrite,
            )
            .with_priority(10),
        );
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Network,
                AccessLevel::ReadWrite,
            )
            .with_priority(10),
        );
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Device,
                AccessLevel::Execute,
            )
            .with_priority(10),
        );
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Compute,
                AccessLevel::Execute,
            )
            .with_priority(10),
        );

        let ai_perms = AIPermissions::permissive(name);

        self.upsert(name, policy, Some(ai_perms))
    }
}

impl PolicyEntry {
    pub fn summary(&self) -> String {
        format!(
            "{} (id: {}, rules: {}, ai_perms: {}, isolate: {})",
            self.name,
            &self.id[..8.min(self.id.len())],
            self.policy.rules.len(),
            self.ai_permissions.is_some(),
            self.ai_permissions
                .as_ref()
                .map(|p| p.isolate)
                .unwrap_or(false)
        )
    }
}
