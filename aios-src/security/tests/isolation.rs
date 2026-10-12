//! Fase 8 acceptance: a restricted model must not reach resources outside its
//! policy.
//!
//! The ROADMAP criterion is *"um modelo com policy restrita não acessa
//! rede/filesystem fora da política; teste automatizado negativo"*. Every test
//! here asserts a **denial** — a test that only proves the happy path cannot
//! fail when the policy is replaced by a permissive stub.
//!
//! Scope note: this exercises the decision and enforcement layer
//! ([`aios_security::Enforcer`]). It is not a proof of kernel-level
//! containment, which needs the Redox `contain` scheme and is not implemented
//! — see `docs/gotchas/security-isolation.md`.

use std::collections::HashMap;

use aios_security::enforce::scope_contains;
use aios_security::{
    AccessLevel, AccessRequest, Enforcer, PolicyEffect, PolicyRule, ResourceType, SecurityPolicy,
};

/// The policy under test: read the model cache, talk to loopback only.
fn restricted_policy() -> SecurityPolicy {
    let mut policy = SecurityPolicy::new("restricted-model");
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
    policy.add_rule(
        PolicyRule::new(
            PolicyEffect::Allow,
            ResourceType::Network,
            AccessLevel::Read,
        )
        .with_condition("host", "localhost"),
    );
    policy
}

fn read(path: &str) -> AccessRequest {
    AccessRequest::new(ResourceType::Filesystem, AccessLevel::Read).with_scope(path)
}

fn write(path: &str) -> AccessRequest {
    AccessRequest::new(ResourceType::Filesystem, AccessLevel::Write).with_scope(path)
}

fn net(host: &str) -> AccessRequest {
    AccessRequest::new(ResourceType::Network, AccessLevel::Read).with_context("host", host)
}

// --- The permitted baseline ----------------------------------------------
//
// These establish that the denials below are caused by the policy and not by
// an enforcer that denies everything.

#[test]
fn baseline_the_model_can_read_its_own_cache() {
    let mut e = Enforcer::new(restricted_policy());
    let decision = e.authorize(&read("/var/lib/ai/models/tinyllama.q4_k_m.gguf"));
    assert!(decision.allowed, "baseline must hold: {decision:?}");
}

#[test]
fn baseline_the_model_can_reach_loopback() {
    let mut e = Enforcer::new(restricted_policy());
    assert!(e.authorize(&net("127.0.0.1")).allowed);
    assert!(e.authorize(&net("localhost")).allowed);
}

// --- Negative: filesystem ------------------------------------------------

#[test]
fn denied_reading_outside_the_model_cache() {
    let mut e = Enforcer::new(restricted_policy());
    for path in [
        "/etc/shadow",
        "/etc/passwd",
        "/home/user/.ssh/id_rsa",
        "/tmp/other.bin",
    ] {
        let decision = e.authorize(&read(path));
        assert!(
            !decision.allowed,
            "reading {path} must be denied: {decision:?}"
        );
    }
}

#[test]
fn denied_writing_anywhere_the_policy_does_not_cover() {
    let mut e = Enforcer::new(restricted_policy());
    // The policy grants read on the cache, never write — not even inside it.
    for path in [
        "/var/lib/ai/models/tinyllama.q4_k_m.gguf",
        "/var/lib/ai/models/other.gguf",
        "/etc/ai-platform",
        "/var/lib/ai/models",
    ] {
        let decision = e.authorize(&write(path));
        assert!(
            !decision.allowed,
            "writing {path} must be denied: {decision:?}"
        );
    }
}

#[test]
fn denied_a_sibling_directory_sharing_the_cache_prefix() {
    // The reason filesystem scope matching is component-wise: a string-prefix
    // check would let "/var/lib/ai/models_backup" pass as "/var/lib/ai/models".
    let mut e = Enforcer::new(restricted_policy());
    for path in [
        "/var/lib/ai/models_backup/secret.gguf",
        "/var/lib/ai/modelsX/secret.gguf",
        "/var/lib/ai/models.bak/secret.gguf",
    ] {
        let decision = e.authorize(&read(path));
        assert!(
            !decision.allowed,
            "reading {path} must be denied: {decision:?}"
        );
    }
}

#[test]
fn denied_traversal_out_of_the_cache() {
    let mut e = Enforcer::new(restricted_policy());
    // An unnormalized path that walks out of the granted directory. This is
    // expected to be denied: the policy is a path allowlist, not a resolver.
    let decision = e.authorize(&read("/var/lib/ai/models/../../../etc/shadow"));
    assert!(!decision.allowed, "traversal must be denied: {decision:?}");
}

// --- Negative: network ---------------------------------------------------

#[test]
fn denied_reaching_hosts_outside_loopback() {
    let mut e = Enforcer::new(restricted_policy());
    for host in [
        "example.com",
        "8.8.8.8",
        "10.0.2.15",
        "0.0.0.0",
        "169.254.169.254",
    ] {
        let decision = e.authorize(&net(host));
        assert!(
            !decision.allowed,
            "reaching {host} must be denied: {decision:?}"
        );
    }
}

#[test]
fn denied_an_unscoped_network_request() {
    // A request with no host in context matches no rule, so it falls through
    // to the default-deny. It must not be treated as "any host allowed".
    let mut e = Enforcer::new(restricted_policy());
    let request = AccessRequest::new(ResourceType::Network, AccessLevel::Read);
    assert!(!e.authorize(&request).allowed);
}

// --- Negative: other resources -------------------------------------------

#[test]
fn denied_devices_system_info_and_ipc() {
    let mut e = Enforcer::new(restricted_policy());
    for (resource, scope) in [
        (ResourceType::Device, "gpu0"),
        (ResourceType::SystemInfo, "/proc/cpuinfo"),
        (ResourceType::Ipc, "/scheme/ipc/bus"),
    ] {
        let request = AccessRequest::new(resource, AccessLevel::Read).with_scope(scope);
        let decision = e.authorize(&request);
        assert!(
            !decision.allowed,
            "{resource:?} must be denied: {decision:?}"
        );
    }
}

// --- Fail-closed behaviour ----------------------------------------------

#[test]
fn require_fails_closed_with_a_structured_error() {
    let mut e = Enforcer::new(restricted_policy());
    let err = e.require(&read("/etc/shadow")).expect_err("must be denied");
    assert_eq!(err.resource, ResourceType::Filesystem);
    assert_eq!(err.access, AccessLevel::Read);
    assert_eq!(err.scope.as_deref(), Some("/etc/shadow"));
}

#[test]
fn an_empty_policy_denies_everything() {
    // Default-deny must hold when no rules exist at all.
    let mut e = Enforcer::new(SecurityPolicy::new("empty"));
    assert!(!e.authorize(&read("/var/lib/ai/models/a.gguf")).allowed);
    assert!(!e.authorize(&net("127.0.0.1")).allowed);
}

#[test]
fn an_explicit_deny_rule_overrides_an_allow_of_equal_priority() {
    // Both rules are priority 0. Without the deny-first tie-break the allow
    // (inserted first) would win and the deny would be dead code.
    let mut policy = SecurityPolicy::new("override");
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
            PolicyEffect::Deny,
            ResourceType::Filesystem,
            AccessLevel::Read,
        )
        .with_scope("/var/lib/ai/models/private"),
    );

    let mut e = Enforcer::new(policy);
    assert!(
        !e.authorize(&read("/var/lib/ai/models/private/a.gguf"))
            .allowed
    );
    assert!(
        e.authorize(&read("/var/lib/ai/models/public/a.gguf"))
            .allowed
    );
}

#[test]
fn a_higher_priority_allow_beats_a_lower_priority_deny() {
    let mut policy = SecurityPolicy::new("priorities");
    policy.add_rule(
        PolicyRule::new(PolicyEffect::Deny, ResourceType::Network, AccessLevel::Read)
            .with_priority(0),
    );
    policy.add_rule(
        PolicyRule::new(
            PolicyEffect::Allow,
            ResourceType::Network,
            AccessLevel::Read,
        )
        .with_priority(10),
    );

    let mut e = Enforcer::new(policy);
    assert!(e.authorize(&net("127.0.0.1")).allowed);
}

#[test]
fn a_permissive_default_effect_does_not_undo_explicit_scope_limits() {
    // Sanity check on the interaction between default_effect and scopes: a
    // permissive default opens unmatched *scopes*, but a scoped deny still
    // wins because the rule is evaluated first.
    let mut policy = SecurityPolicy::new("permissive-default");
    policy.default_effect = PolicyEffect::Allow;
    policy.add_rule(
        PolicyRule::new(
            PolicyEffect::Deny,
            ResourceType::Filesystem,
            AccessLevel::Read,
        )
        .with_scope("/etc")
        .with_priority(5),
    );

    let mut e = Enforcer::new(policy);
    assert!(!e.authorize(&read("/etc/shadow")).allowed);
    assert!(e.authorize(&read("/var/lib/ai/models/a.gguf")).allowed);
}

#[test]
fn the_runtime_permission_path_hides_the_same_holes_as_the_policy_path() {
    // `PermissionSet` is what the running system checks, and it does its own
    // scope comparison. If that comparison regressed to a prefix check while
    // the policy layer stayed correct, these asserts would be the only thing
    // catching it — so they are deliberately the same cases.
    let policy = restricted_policy();
    let set = policy.to_permission_set();

    assert!(set.check(
        ResourceType::Filesystem,
        AccessLevel::Read,
        Some("/var/lib/ai/models/a.gguf")
    ));
    for path in [
        "/var/lib/ai/models_backup/a.gguf",
        "/var/lib/ai/modelsX/a.gguf",
        "/var/lib/ai/models/../../../etc/shadow",
        "/etc/shadow",
    ] {
        assert!(
            !set.check(ResourceType::Filesystem, AccessLevel::Read, Some(path)),
            "runtime path must deny {path}"
        );
    }
}

// --- Audit --------------------------------------------------------------

#[test]
fn every_denial_is_recorded() {
    let mut e = Enforcer::new(restricted_policy());
    e.authorize(&read("/var/lib/ai/models/a.gguf"));
    e.authorize(&read("/etc/shadow"));
    e.authorize(&write("/etc/passwd"));
    e.authorize(&net("example.com"));

    let log = e.audit_log();
    assert_eq!(log.len(), 4);
    assert_eq!(log.iter().filter(|d| d.allowed).count(), 1);
    assert_eq!(e.denied_count(), 3);
    // A denial names the rule that produced it, when one did.
    assert!(log.iter().all(|d| d.reason.contains("policy")));
}

// --- Conversion ---------------------------------------------------------

#[test]
fn an_unscoped_allow_rule_survives_conversion_to_a_permission_set() {
    // Regression: converting a scope-less allow used to write an empty-string
    // scope, which `PermissionRule::allows` treats as "matches nothing", so the
    // allow silently became a deny.
    let mut policy = SecurityPolicy::new("unscoped");
    policy.add_rule(PolicyRule::new(
        PolicyEffect::Allow,
        ResourceType::Network,
        AccessLevel::Read,
    ));

    let set = policy.to_permission_set();
    assert_eq!(set.rules.len(), 1);
    assert!(
        set.check(ResourceType::Network, AccessLevel::Read, None),
        "unscoped allow must still allow an unscoped request"
    );
}

#[test]
fn a_scoped_allow_keeps_its_scope_through_conversion() {
    let policy = restricted_policy();
    let set = policy.to_permission_set();
    assert!(set.check(
        ResourceType::Filesystem,
        AccessLevel::Read,
        Some("/var/lib/ai/models/a.gguf")
    ));
    assert!(!set.check(
        ResourceType::Filesystem,
        AccessLevel::Read,
        Some("/etc/shadow")
    ));
}

// --- Scope helper -------------------------------------------------------

#[test]
fn scope_containment_is_component_wise_for_paths() {
    assert!(scope_contains(
        ResourceType::Filesystem,
        "/var/lib/ai",
        "/var/lib/ai/models/a.gguf"
    ));
    assert!(!scope_contains(
        ResourceType::Filesystem,
        "/var/lib/ai",
        "/var/lib/aiother/a.gguf"
    ));
    assert!(!scope_contains(
        ResourceType::Filesystem,
        "/var/lib/ai",
        "/var/lib"
    ));
}

#[test]
fn an_empty_rule_scope_contains_everything() {
    assert!(scope_contains(ResourceType::Filesystem, "", "/any/path"));
}

#[test]
fn context_conditions_are_honoured() {
    let mut e = Enforcer::new(restricted_policy());
    // Right rule shape, wrong condition value.
    let wrong = AccessRequest::new(ResourceType::Network, AccessLevel::Write)
        .with_context("host", "127.0.0.1");
    assert!(!e.authorize(&wrong).allowed, "write is not granted at all");

    // A context that satisfies nothing extra still needs the host condition.
    let mut context = HashMap::new();
    context.insert("unrelated".to_string(), "value".to_string());
    let request = AccessRequest {
        resource: ResourceType::Network,
        access: AccessLevel::Read,
        scope: None,
        context,
    };
    assert!(!e.authorize(&request).allowed);
}
