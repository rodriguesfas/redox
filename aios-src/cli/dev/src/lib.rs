//! `aios-dev`: the AIOS Developer CLI (Fase 2).
//!
//! Split into a library so the interesting behaviour — manifest validation,
//! scaffolding and the environment doctor — is unit-testable without spawning
//! the binary. The `dev` binary in `main.rs` is a thin argument parser over
//! this crate.

pub mod doctor;
pub mod project;
pub mod scaffold;

pub use doctor::{Check, Report, Status};
pub use project::{ProjectManifest, ProjectPermissions, ValidationError, TEMPLATES};
pub use scaffold::{resolve_workspace, scaffold, ScaffoldError, Scaffolded};
