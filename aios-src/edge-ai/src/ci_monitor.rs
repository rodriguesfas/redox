//! CI/Deploy monitor for polling deployment status and sending desktop notifications.

use crate::http::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// CI/Deploy status from the monitoring endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiStatus {
    pub status: String,
    pub pipeline: Option<String>,
    pub build_number: Option<u64>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_seconds: Option<u64>,
    pub result: Option<String>, // success, failed, running, pending
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub url: Option<String>,
}

/// Configuration for CI monitoring
#[derive(Debug, Clone)]
pub struct CiMonitorConfig {
    pub base_url: String,
    pub endpoint_path: String,
    pub poll_interval_secs: u64,
    pub notify_on_success: bool,
    pub notify_on_failure: bool,
    pub notify_on_start: bool,
    pub cookie_file: Option<String>, // Path to cookie file for auth
}

impl Default for CiMonitorConfig {
    fn default() -> Self {
        Self {
            base_url: "http://10.8.0.9:15201".to_string(),
            endpoint_path: "/ci".to_string(),
            poll_interval_secs: 30,
            notify_on_success: true,
            notify_on_failure: true,
            notify_on_start: false,
            cookie_file: None,
        }
    }
}

/// CI Monitor for polling and notifications
pub struct CiMonitor {
    client: Client,
    config: CiMonitorConfig,
    last_status: Option<CiStatus>,
}

impl CiMonitor {
    /// Create a new CI monitor
    pub fn new(config: CiMonitorConfig) -> Result<Self, String> {
        // Strip http:// or https:// prefix if present
        let base_url = config
            .base_url
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let client = if let Some(cookie_path) = &config.cookie_file {
            let cookies = load_cookies_from_file(cookie_path)?;
            let cookie_header = cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            Client::with_cookie(base_url, cookie_header)?
        } else {
            Client::from_base(base_url)?
        };
        Ok(Self {
            client,
            config,
            last_status: None,
        })
    }

    /// Create with default config (can be overridden)
    pub fn with_defaults(base_url: Option<String>) -> Result<Self, String> {
        let mut config = CiMonitorConfig::default();
        if let Some(url) = base_url {
            config.base_url = url;
        }
        Self::new(config)
    }

    /// Fetch current CI status from the endpoint
    pub fn fetch_status(&self) -> Result<CiStatus, String> {
        let (status_code, body) = self
            .client
            .get(&self.config.endpoint_path)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if status_code == 302 {
            return Err(
                "Received 302 redirect - likely oauth2-proxy authentication required. \
                Provide cookies via --cookie-file or authenticate first in browser."
                    .to_string(),
            );
        }

        if status_code >= 400 {
            return Err(format!(
                "HTTP {status_code}: {}",
                String::from_utf8_lossy(&body)
            ));
        }

        let ci_status: CiStatus = serde_json::from_slice(&body)
            .map_err(|e| format!("Failed to parse CI status JSON: {e}"))?;

        Ok(ci_status)
    }

    /// Check if status has changed significantly
    fn status_changed(&self, new: &CiStatus) -> bool {
        match &self.last_status {
            None => true,
            Some(old) => {
                old.result != new.result
                    || old.status != new.status
                    || old.finished_at != new.finished_at
            }
        }
    }

    /// Send desktop notification using notify-send
    pub fn send_notification(&self, title: &str, body: &str, urgency: &str) -> Result<(), String> {
        let output = std::process::Command::new("notify-send")
            .args(["-u", urgency, "-a", "AIOS CI Monitor", title, body])
            .output()
            .map_err(|e| format!("Failed to execute notify-send: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("notify-send failed: {stderr}"));
        }
        Ok(())
    }

    /// Determine notification urgency based on status
    fn urgency_for_status(result: &Option<String>) -> &'static str {
        match result.as_deref() {
            Some("failed") | Some("error") => "critical",
            Some("success") => "normal",
            _ => "low",
        }
    }

    /// Format notification message from CI status
    fn format_notification(&self, status: &CiStatus) -> (String, String) {
        let pipeline = status.pipeline.as_deref().unwrap_or("CI/Deploy");
        let build = status
            .build_number
            .map(|n| format!("#{}", n))
            .unwrap_or_default();
        let result = status.result.as_deref().unwrap_or("unknown");
        let commit = status.commit.as_deref().unwrap_or("unknown");
        let short_commit = if commit.len() > 8 {
            &commit[..8]
        } else {
            commit
        };

        let title = format!("{} {}: {}", pipeline, build, result.to_uppercase());
        let body = format!(
            "Commit: {}\nBranch: {}\nDuration: {}s",
            short_commit,
            status.branch.as_deref().unwrap_or("unknown"),
            status.duration_seconds.unwrap_or(0)
        );

        (title, body)
    }

    /// Poll once and send notification if status changed
    pub fn poll_once(&mut self) -> Result<bool, String> {
        let status = self.fetch_status()?;
        let changed = self.status_changed(&status);

        if changed {
            let (title, body) = self.format_notification(&status);
            let urgency = Self::urgency_for_status(&status.result);

            // Check if we should notify for this status
            let should_notify = match status.result.as_deref() {
                Some("success") => self.config.notify_on_success,
                Some("failed") | Some("error") => self.config.notify_on_failure,
                Some("running") | Some("pending") => self.config.notify_on_start,
                _ => false,
            };

            if should_notify {
                self.send_notification(&title, &body, urgency)?;
            }

            self.last_status = Some(status);
        }

        Ok(changed)
    }

    /// Run continuous monitoring loop
    pub fn run(&mut self) -> Result<(), String> {
        println!(
            "Starting CI monitor for {} (polling every {}s)...",
            self.config.base_url, self.config.poll_interval_secs
        );
        println!("Press Ctrl+C to stop");

        // Initial fetch
        match self.poll_once() {
            Ok(true) => println!("Initial status fetched"),
            Ok(false) => println!("No initial status change"),
            Err(e) => {
                eprintln!("Initial fetch failed: {e}");
                eprintln!("Continuing to poll...");
            }
        }

        loop {
            std::thread::sleep(Duration::from_secs(self.config.poll_interval_secs));
            match self.poll_once() {
                Ok(true) => println!("Status changed, notification sent"),
                Ok(false) => {} // No change
                Err(e) => eprintln!("Poll error: {e}"),
            }
        }
    }
}

/// Load cookies from a Netscape format cookie file
pub fn load_cookies_from_file(path: &str) -> Result<Vec<(String, String)>, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read cookie file: {e}"))?;

    let mut cookies = Vec::new();
    for line in content.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 7 {
            let name = parts[5].to_string();
            let value = parts[6].to_string();
            cookies.push((name, value));
        }
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urgency_for_status() {
        assert_eq!(
            CiMonitor::urgency_for_status(&Some("failed".to_string())),
            "critical"
        );
        assert_eq!(
            CiMonitor::urgency_for_status(&Some("error".to_string())),
            "critical"
        );
        assert_eq!(
            CiMonitor::urgency_for_status(&Some("success".to_string())),
            "normal"
        );
        assert_eq!(
            CiMonitor::urgency_for_status(&Some("running".to_string())),
            "low"
        );
        assert_eq!(CiMonitor::urgency_for_status(&None), "low");
    }

    #[test]
    fn test_format_notification() {
        let monitor = CiMonitor {
            client: Client::from_base("localhost:8989").unwrap(),
            config: CiMonitorConfig::default(),
            last_status: None,
        };

        let status = CiStatus {
            status: "completed".to_string(),
            pipeline: Some("deploy".to_string()),
            build_number: Some(42),
            started_at: Some("2026-01-01T00:00:00Z".to_string()),
            finished_at: Some("2026-01-01T00:05:00Z".to_string()),
            duration_seconds: Some(300),
            result: Some("success".to_string()),
            commit: Some("abc123def456".to_string()),
            branch: Some("main".to_string()),
            url: Some("http://10.8.0.9:15201/ci/42".to_string()),
        };

        let (title, body) = monitor.format_notification(&status);
        assert!(title.contains("deploy"));
        assert!(title.contains("#42"));
        assert!(title.contains("SUCCESS"));
        assert!(body.contains("abc123de"));
        assert!(body.contains("main"));
        assert!(body.contains("300s"));
    }
}
