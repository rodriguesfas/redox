//! Runtime authorization for AI models (Fase 8).
//!
//! The policy engine in [`crate::policy`] decides *what* is permitted; this
//! module is the point where that decision is actually consulted, recorded and
//! able to stop an action. It is deliberately independent of the `contain`
//! scheme: the decision logic is fully exercisable off-Redox, which is what
//! makes the negative tests in `tests/isolation.rs` meaningful.
//!
//! What this does **not** do is intercept syscalls. A denied decision is only
//! enforced if the caller honours it. Kernel-level enforcement needs Redox's
//! `contain` scheme, which is not implemented — see
//! `docs/gotchas/security-isolation.md`.

use std::collections::HashMap;
use std::fmt;

use crate::permissions::{AccessLevel, ResourceType};
use crate::policy::{PolicyDecision, PolicyEffect, SecurityPolicy};

/// A request for one resource, made by a model or service under policy.
#[derive(Debug, Clone)]
pub struct AccessRequest {
    pub resource: ResourceType,
    pub access: AccessLevel,
    /// The concrete path, host or device the access targets. `None` means the
    /// request is not scoped (e.g. "any network inbound").
    pub scope: Option<String>,
    /// Request attributes rules can condition on, e.g. `port` or `model`.
    pub context: HashMap<String, String>,
}

impl AccessRequest {
    pub fn new(resource: ResourceType, access: AccessLevel) -> Self {
        AccessRequest {
            resource,
            access,
            scope: None,
            context: HashMap::new(),
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

/// Returned when a request is denied, so callers can fail closed with the
/// reason attached instead of a bare `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessDenied {
    pub resource: ResourceType,
    pub access: AccessLevel,
    pub scope: Option<String>,
    pub reason: String,
}

impl fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "access denied: {:?} {:?}", self.resource, self.access)?;
        if let Some(scope) = &self.scope {
            write!(f, " on '{scope}'")?;
        }
        write!(f, " ({})", self.reason)
    }
}

impl std::error::Error for AccessDenied {}

/// Consults a policy on behalf of a model, recording every decision.
///
/// The audit log is the point: a security control nobody can inspect is
/// indistinguishable from one that is not running.
#[derive(Debug)]
pub struct Enforcer {
    policy: SecurityPolicy,
    audit: Vec<PolicyDecision>,
    /// Cap on retained decisions so a long-lived daemon cannot grow unbounded.
    audit_capacity: usize,
}

impl Enforcer {
    /// Build an enforcer. The policy's `default_effect` decides anything no
    /// rule matches, which is `Deny` for a policy built with
    /// [`SecurityPolicy::new`].
    pub fn new(policy: SecurityPolicy) -> Self {
        Enforcer {
            policy,
            audit: Vec::new(),
            audit_capacity: 1024,
        }
    }

    pub fn with_audit_capacity(mut self, capacity: usize) -> Self {
        self.audit_capacity = capacity.max(1);
        self
    }

    pub fn policy(&self) -> &SecurityPolicy {
        &self.policy
    }

    /// Decide a request, recording the outcome.
    pub fn authorize(&mut self, request: &AccessRequest) -> PolicyDecision {
        let (effect, rule_id) = self.policy.evaluate_with_rule(
            request.resource,
            request.access,
            request.scope.as_deref(),
            &request.context,
        );
        let decision = match effect {
            PolicyEffect::Allow => PolicyDecision::allow(rule_id, "allowed by policy"),
            PolicyEffect::Deny => PolicyDecision::deny(
                rule_id,
                "no rule grants this access; policy default is deny",
            ),
        };
        self.record(decision.clone());
        decision
    }

    /// Decide a request, failing closed on denial.
    pub fn require(&mut self, request: &AccessRequest) -> Result<(), AccessDenied> {
        if self.authorize(request).allowed {
            Ok(())
        } else {
            Err(AccessDenied {
                resource: request.resource,
                access: request.access,
                scope: request.scope.clone(),
                reason: "denied by policy".to_string(),
            })
        }
    }

    fn record(&mut self, decision: PolicyDecision) {
        self.audit.push(decision);
        if self.audit.len() > self.audit_capacity {
            // Drop the oldest decisions rather than refusing to record new
            // ones: a full audit buffer must not silently disable auditing.
            let excess = self.audit.len() - self.audit_capacity;
            self.audit.drain(0..excess);
        }
    }

    /// Every decision made so far, oldest first.
    pub fn audit_log(&self) -> &[PolicyDecision] {
        &self.audit
    }

    /// Count of denied decisions, for metrics and for the negative tests.
    pub fn denied_count(&self) -> usize {
        self.audit.iter().filter(|d| !d.allowed).count()
    }
}

/// True when `request_scope` falls inside `rule_scope`.
///
/// For filesystem paths this is a component-wise check on the *normalized*
/// path, not a string prefix. Two things follow from that, and both are
/// security-relevant:
///
/// * `/var/lib/ai/models` must not authorize `/var/lib/ai/models_backup`,
///   which a naive `starts_with` would allow.
/// * `/var/lib/ai/models/../../../etc/shadow` must not be authorized by a
///   grant on `/var/lib/ai/models`, even though the string starts with it.
///   The `..` segments are collapsed first, so the request is compared as
///   `/etc/shadow` and falls outside.
///
/// Normalization is lexical: no filesystem access, so a symlink pointing out
/// of an allowed directory is not caught here. Resolving that needs the
/// caller's real path, not the policy layer.
pub fn scope_contains(resource: ResourceType, rule_scope: &str, request_scope: &str) -> bool {
    if resource != ResourceType::Filesystem {
        return request_scope.starts_with(rule_scope);
    }
    let rule = normalize_path(rule_scope);
    if rule.is_empty() || rule == "/" {
        return true;
    }
    let request = normalize_path(request_scope);
    request == rule || request.starts_with(&format!("{rule}/"))
}

/// Collapse `.` and `..` in a path without touching the filesystem.
///
/// Leading `..` segments on a relative path are preserved, since dropping
/// them would silently change the meaning of a relative scope.
fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();

    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => match parts.last() {
                // Only pop a segment we can actually climb out of.
                Some(last) if *last != ".." => {
                    parts.pop();
                }
                _ if absolute => {
                    // `/..` is `/`; nothing to climb.
                }
                _ => parts.push(".."),
            },
            other => parts.push(other),
        }
    }

    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Human-readable summary of what a policy permits, for `aios-security show`.
pub fn describe(policy: &SecurityPolicy) -> String {
    let mut out = format!(
        "policy '{}' v{} (default: {:?})\n",
        policy.name, policy.version, policy.default_effect
    );
    let mut rules = policy.rules.iter().collect::<Vec<_>>();
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    for rule in rules {
        let scope = rule.scope.as_deref().unwrap_or("*");
        let conditions: Vec<String> = rule
            .conditions
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let cond = if conditions.is_empty() {
            String::new()
        } else {
            format!(" [{}]", conditions.join(", "))
        };
        out.push_str(&format!(
            "  {:?} {:?} {:?} on {scope} (priority {}){cond}\n",
            rule.effect, rule.resource, rule.access, rule.priority
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PolicyRule;

    fn restricted() -> SecurityPolicy {
        let mut policy = SecurityPolicy::new("restricted");
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Filesystem,
                AccessLevel::Read,
            )
            .with_scope("/var/lib/ai/models"),
        );
        policy.add_rule(
            PolicyRule::new(
                PolicyEffect::Allow,
                ResourceType::Network,
                AccessLevel::Read,
            )
            .with_condition("host", "127.0.0.1"),
        );
        policy
    }

    #[test]
    fn allows_access_inside_the_granted_scope() {
        let mut e = Enforcer::new(restricted());
        let request = AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
            .with_scope("/var/lib/ai/models/tinyllama.gguf");
        assert!(e.authorize(&request).allowed);
    }

    #[test]
    fn denies_filesystem_access_outside_the_granted_scope() {
        let mut e = Enforcer::new(restricted());
        let request = AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
            .with_scope("/etc/shadow");
        let decision = e.authorize(&request);
        assert!(!decision.allowed, "{decision:?}");
        assert_eq!(e.denied_count(), 1);
    }

    #[test]
    fn a_filesystem_grant_does_not_cover_a_sibling_with_a_shared_prefix() {
        // The regression this guards: a naive `starts_with` would treat
        // "/var/lib/ai/models_backup" as inside "/var/lib/ai/models".
        assert!(scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models/tinyllama.gguf"
        ));
        assert!(!scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models_backup/secret.gguf"
        ));
        assert!(!scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/modelsX"
        ));
        // Exact match and a trailing slash on the rule both behave.
        assert!(scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models"
        ));
        assert!(scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models/",
            "/var/lib/ai/models/a"
        ));
    }

    #[test]
    fn dot_dot_segments_cannot_walk_out_of_a_granted_directory() {
        assert!(!scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models/../../../etc/shadow"
        ));
        assert!(!scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models/../models_backup/x"
        ));
        // A `..` that stays inside remains allowed.
        assert!(scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai/models/sub/../a.gguf"
        ));
        // Redundant separators and `.` are normalized away, not treated as
        // distinct directories.
        assert!(scope_contains(
            ResourceType::Filesystem,
            "/var/lib/ai/models",
            "/var/lib/ai//models/./a.gguf"
        ));
    }

    #[test]
    fn normalization_keeps_leading_parent_segments_on_relative_paths() {
        // Dropping these would silently retarget a relative scope.
        assert_eq!(normalize_path("../a/b"), "../a/b");
        assert_eq!(normalize_path("/a/../b"), "/b");
        assert_eq!(normalize_path("/.."), "/");
        assert_eq!(normalize_path("a/./b/../c"), "a/c");
    }

    #[test]
    fn non_filesystem_scopes_keep_prefix_semantics() {
        assert!(scope_contains(
            ResourceType::Network,
            "127.0.0.1",
            "127.0.0.1"
        ));
        assert!(scope_contains(
            ResourceType::Network,
            "10.0.2.",
            "10.0.2.15"
        ));
        assert!(!scope_contains(
            ResourceType::Network,
            "10.0.2.",
            "10.0.3.15"
        ));
    }

    #[test]
    fn denies_network_outside_the_allowed_host() {
        let mut e = Enforcer::new(restricted());
        let request = AccessRequest::new(ResourceType::Network, AccessLevel::Read)
            .with_context("host", "example.com");
        assert!(!e.authorize(&request).allowed);
    }

    #[test]
    fn require_returns_a_structured_denial() {
        let mut e = Enforcer::new(restricted());
        let request = AccessRequest::new(ResourceType::Filesystem, AccessLevel::Write)
            .with_scope("/etc/passwd");
        let err = e.require(&request).unwrap_err();
        assert_eq!(err.resource, ResourceType::Filesystem);
        assert_eq!(err.access, AccessLevel::Write);
        assert_eq!(err.scope.as_deref(), Some("/etc/passwd"));
        assert!(err.to_string().contains("/etc/passwd"));
    }

    #[test]
    fn require_succeeds_for_permitted_access() {
        let mut e = Enforcer::new(restricted());
        let request = AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
            .with_scope("/var/lib/ai/models/tinyllama.gguf");
        assert!(e.require(&request).is_ok());
    }

    #[test]
    fn audit_log_records_every_decision_in_order() {
        let mut e = Enforcer::new(restricted());
        e.authorize(
            &AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
                .with_scope("/var/lib/ai/models/a.gguf"),
        );
        e.authorize(
            &AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
                .with_scope("/etc/shadow"),
        );
        let log = e.audit_log();
        assert_eq!(log.len(), 2);
        assert!(log[0].allowed);
        assert!(!log[1].allowed);
        assert!(log[0].matched_rule.is_some(), "allow should name its rule");
    }

    #[test]
    fn audit_buffer_is_bounded_but_keeps_the_newest_decisions() {
        let mut e = Enforcer::new(restricted()).with_audit_capacity(3);
        for i in 0..10 {
            e.authorize(
                &AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read)
                    .with_scope(format!("/var/lib/ai/models/{i}.gguf")),
            );
        }
        let log = e.audit_log();
        assert_eq!(log.len(), 3, "audit must stay bounded");
        assert_eq!(e.denied_count(), 0);
    }

    #[test]
    fn unknown_resources_are_denied_by_default() {
        let mut e = Enforcer::new(restricted());
        let request =
            AccessRequest::new(ResourceType::Device, AccessLevel::Execute).with_scope("gpu0");
        assert!(!e.authorize(&request).allowed);
    }

    #[test]
    fn describe_lists_the_rules() {
        let text = describe(&restricted());
        assert!(text.contains("policy 'restricted'"));
        assert!(text.contains("/var/lib/ai/models"));
    }
}
