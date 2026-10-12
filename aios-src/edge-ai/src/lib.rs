//! edge-ai: Edge AI OS runtime (Fase 5).
//!
//! A local HTTP daemon exposing a small JSON API (health, models, infer,
//! benchmark, logs, metrics) plus an HTML control panel, a `edge` CLI client
//! and an `ai-monitor` metrics sampler with a history ring. Dependency-free
//! micro HTTP (no external server crate) so it cross-compiles cleanly for the
//! `*-unknown-redox` targets; inference reuses the aios-inference Candle CPU
//! backend (ADR-003).

pub mod api;
pub mod ci_monitor;
pub mod daemon;
pub mod http;
pub mod metrics;
pub mod panel;
pub mod runtime;

pub use runtime::Runtime;

/// Default port for the Edge AI OS daemon.
pub const DEFAULT_PORT: u16 = 8989;

/// Default bind address for the daemon (reachable from the dev notebook).
pub const DEFAULT_BIND: &str = "0.0.0.0";

/// Default host the `edge` CLI talks to.
pub const DEFAULT_CLIENT_BASE: &str = "127.0.0.1:8989";
