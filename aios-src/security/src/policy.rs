//! Security Policy Engine
//!
//! Policy evaluation and enforcement for AI model isolation.

use crate::permissions::{AccessLevel, PermissionRule, PermissionSet, ResourceType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Policy effect (allow/deny)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyEffect {
    #[default]
    Deny,
    Allow,
}

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub effect: PolicyEffect,
    pub resource: ResourceType,
    pub access: AccessLevel,
    pub scope: Option<String>,
    pub conditions: HashMap<String, String>,
    pub priority: i32, // Higher = evaluated first
}

impl PolicyRule {
    pub fn new(effect: PolicyEffect, resource: ResourceType, access: AccessLevel) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            effect,
            resource,
            access,
            scope: None,
            conditions: HashMap::new(),
            priority: 0,
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_condition(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.conditions.insert(key.into(), value.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if this rule matches the request
    pub fn matches(
        &self,
        resource: ResourceType,
        access: AccessLevel,
        scope: Option<&str>,
        context: &HashMap<String, String>,
    ) -> bool {
        if self.resource != resource {
            return false;
        }

        // Check access level matches
        let access_match = matches!(
            (self.access, access),
            (AccessLevel::ReadWrite, AccessLevel::Read)
                | (AccessLevel::ReadWrite, AccessLevel::Write)
                | (AccessLevel::ReadWrite, AccessLevel::ReadWrite)
                | (AccessLevel::Read, AccessLevel::Read)
                | (AccessLevel::Write, AccessLevel::Write)
                | (AccessLevel::Execute, AccessLevel::Execute)
        );
        if !access_match {
            return false;
        }

        // Check scope. A rule without a scope matches any scope; a rule with
        // one requires a request scope inside it. Scope containment is
        // component-wise for filesystem paths, so a grant on
        // /var/lib/ai/models does not cover /var/lib/ai/models_backup.
        if let Some(ref s) = self.scope {
            match scope {
                None => return false,
                Some(sc) => {
                    if !crate::enforce::scope_contains(self.resource, s, sc) {
                        return false;
                    }
                }
            }
        }

        // Check conditions
        for (k, v) in &self.conditions {
            if context.get(k) != Some(v) {
                return false;
            }
        }

        true
    }
}

/// Complete security policy
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub name: String,
    pub version: String,
    pub rules: Vec<PolicyRule>,
    pub default_effect: PolicyEffect,
}

impl SecurityPolicy {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "1.0".to_string(),
            rules: Vec::new(),
            default_effect: PolicyEffect::Deny,
        }
    }

    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.rules.push(rule);
        self.sort_rules();
    }

    /// Order rules for evaluation: highest priority first, and at equal
    /// priority `Deny` before `Allow`.
    ///
    /// The tie-break matters: `evaluate` returns the first match, so without it
    /// two equally-ranked rules resolve by insertion order and a later `deny`
    /// would be silently ineffective.
    fn sort_rules(&mut self) {
        self.rules.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| deny_rank(a).cmp(&deny_rank(b)))
        });
    }

    /// Re-assert the evaluation order after the rules are edited directly
    /// (e.g. loaded from disk), so evaluation never depends on file order.
    pub fn normalize(&mut self) {
        self.sort_rules();
    }

    pub fn allow(&mut self, resource: ResourceType, access: AccessLevel) -> &mut Self {
        self.add_rule(PolicyRule::new(PolicyEffect::Allow, resource, access));
        self
    }

    pub fn deny(&mut self, resource: ResourceType, access: AccessLevel) -> &mut Self {
        self.add_rule(PolicyRule::new(PolicyEffect::Deny, resource, access));
        self
    }

    /// Evaluate a request against this policy
    pub fn evaluate(
        &self,
        resource: ResourceType,
        access: AccessLevel,
        scope: Option<&str>,
        context: &HashMap<String, String>,
    ) -> PolicyEffect {
        for rule in &self.rules {
            if rule.matches(resource, access, scope, context) {
                return rule.effect;
            }
        }
        self.default_effect
    }

    /// Evaluate and report which rule decided, for audit trails.
    pub fn evaluate_with_rule(
        &self,
        resource: ResourceType,
        access: AccessLevel,
        scope: Option<&str>,
        context: &HashMap<String, String>,
    ) -> (PolicyEffect, Option<String>) {
        for rule in &self.rules {
            if rule.matches(resource, access, scope, context) {
                return (rule.effect, Some(rule.id.clone()));
            }
        }
        (self.default_effect, None)
    }

    /// Convert to PermissionSet for runtime enforcement.
    ///
    /// A rule with no scope becomes a scope-less rule, which matches any
    /// request. Writing `Some("")` here instead would make
    /// `PermissionRule::allows` reject every unscoped request, silently
    /// inverting an allow into a deny.
    pub fn to_permission_set(&self) -> PermissionSet {
        let mut set = PermissionSet::new();
        for rule in &self.rules {
            if rule.effect == PolicyEffect::Allow {
                let mut perm = PermissionRule::new(rule.resource, rule.access);
                if let Some(scope) = &rule.scope {
                    perm = perm.with_scope(scope.clone());
                }
                set.add_rule(perm);
            }
        }
        set
    }
}

/// Sort helper: `Deny` sorts before `Allow`.
fn deny_rank(rule: &PolicyRule) -> i32 {
    match rule.effect {
        PolicyEffect::Deny => 0,
        PolicyEffect::Allow => 1,
    }
}

/// Policy decision result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub matched_rule: Option<String>,
    pub effect: PolicyEffect,
    pub reason: String,
}

impl PolicyDecision {
    pub fn allow(rule_id: Option<String>, reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            matched_rule: rule_id,
            effect: PolicyEffect::Allow,
            reason: reason.into(),
        }
    }

    pub fn deny(rule_id: Option<String>, reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            matched_rule: rule_id,
            effect: PolicyEffect::Deny,
            reason: reason.into(),
        }
    }
}
