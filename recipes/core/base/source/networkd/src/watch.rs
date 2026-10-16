//! Keeping the stack in step with the configuration while the system runs.
//!
//! The daemon applies the configuration at boot, but the system does not stand
//! still: the network stack comes up after the daemon that reads it, an
//! interface appears when a driver is loaded, and a configuration is edited by
//! the person who owns the machine. The daemon therefore looks again from time
//! to time — and a look that finds nothing to do costs one read of a directory
//! and a few small files, which is what makes looking often the cheap and boring
//! answer.
//!
//! Everything here is pure and runs on the host: the daemon is only the thread
//! that calls [`Watcher::look`] in a loop.
//!
//! The interval is the same contract as the one `resolved` follows
//! (`NETWORKD_INTERVAL`: seconds, `0` turns watching off, anything else is
//! reported), so the two daemons of the boot are told how to behave in the same
//! words.

use std::time::Duration;

use crate::state::{lock, Application, NetworkdError, SharedNetworkd};

/// Environment variable that says how often the configuration is looked at.
pub const ENV_INTERVAL: &str = "NETWORKD_INTERVAL";

/// How often the configuration is looked at when the environment says nothing.
///
/// Five seconds is a compromise a network can live with: long enough that an
/// idle system is not woken for nothing, short enough that an interface which
/// just appeared is configured by the time a program asks for the network.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);

/// Reads the interval a value asks for.
///
/// The value is a number of seconds. `0` turns the watching off, which is what a
/// system whose configuration is applied once wants. Anything that is not a
/// number is an error for the caller to report: a typo in the environment must
/// not silently leave a configuration that is never applied.
pub fn parse_interval(value: &str) -> Result<Option<Duration>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("the interval is empty".to_owned());
    }

    match value.parse::<u64>() {
        Ok(0) => Ok(None),
        Ok(seconds) => Ok(Some(Duration::from_secs(seconds))),
        Err(err) => Err(format!("{value:?} is not a number of seconds: {err}")),
    }
}

/// Looks at the configuration, and writes the stack when it says something new.
pub struct Watcher {
    state: SharedNetworkd,
    interval: Duration,
    looks: u64,
}

impl Watcher {
    /// Watches a configuration, looking every `interval`.
    pub fn new(state: SharedNetworkd, interval: Duration) -> Self {
        Self {
            state,
            interval,
            looks: 0,
        }
    }

    /// How long there is between two looks.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// How many times the configuration was looked at.
    pub fn looks(&self) -> u64 {
        self.looks
    }

    /// One look at the configuration.
    ///
    /// A stack that is not mounted at all is not a failure: the network comes up
    /// after this daemon does. In that case nothing is written and the next look
    /// tries again, which is the whole point of watching. What is answered is
    /// the application when the stack changed, so a caller can say so.
    pub fn look(&mut self) -> Result<Option<Application>, NetworkdError> {
        self.looks += 1;

        let mut networkd = lock(&self.state);
        if !networkd.netcfg().is_mounted() {
            return Ok(None);
        }

        let application = networkd.apply()?;
        Ok(if application.changed {
            Some(application)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::netcfg::Netcfg;
    use crate::state::{shared, Networkd};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("networkd-watch-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stack with one interface, which a configuration is watched against.
    fn watched(dir: &Path, mounted: bool) -> (SharedNetworkd, PathBuf) {
        let netcfg = dir.join(if mounted { "netcfg" } else { "absent" });
        if mounted {
            for path in ["ifaces/eth0/mac", "ifaces/eth0/addr/set"] {
                let file = netcfg.join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(&file, "").unwrap();
            }
        }

        let config_dir = dir.join("networkd");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("10-eth0.network"),
            "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n",
        )
        .unwrap();

        let state = shared(Networkd::new(
            config_dir,
            Netcfg::new(netcfg.to_string_lossy().into_owned()),
        ));

        (state, netcfg)
    }

    #[test]
    fn a_look_applies_the_configuration_when_the_stack_says_something_new() {
        let dir = scratch("look");
        let (state, netcfg) = watched(&dir, true);
        let mut watcher = Watcher::new(state, DEFAULT_INTERVAL);

        assert_eq!(watcher.interval(), DEFAULT_INTERVAL);
        assert_eq!(watcher.looks(), 0);

        // The stack is there and the interface is not configured: it is.
        let application = watcher.look().unwrap().expect("there is work to do");
        assert!(application.changed);
        assert_eq!(
            fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
            "192.168.1.50/24\n"
        );
        assert_eq!(watcher.looks(), 1);

        // The stack agrees: a look finds nothing to do.
        assert!(watcher.look().unwrap().is_none());
        assert_eq!(watcher.looks(), 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_look_that_finds_no_stack_is_not_a_failure() {
        let dir = scratch("no-stack");
        let (state, netcfg) = watched(&dir, false);
        let mut watcher = Watcher::new(state, DEFAULT_INTERVAL);

        // The stack is not mounted: the network comes up after this daemon.
        assert!(watcher.look().unwrap().is_none());
        assert!(!netcfg.exists(), "nothing to write, and nothing written");

        // The stack arrives: the same watcher carries the configuration into it,
        // without being told.
        for path in ["ifaces/eth0/mac", "ifaces/eth0/addr/set"] {
            let file = netcfg.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "").unwrap();
        }
        assert!(watcher.look().unwrap().is_some());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_interval_comes_from_the_environment_when_it_is_a_number() {
        assert_eq!(parse_interval("5"), Ok(Some(Duration::from_secs(5))));
        assert_eq!(parse_interval(" 30 \n"), Ok(Some(Duration::from_secs(30))));
        assert_eq!(parse_interval("0"), Ok(None), "zero turns the watching off");

        for value in ["", "  ", "five", "5s", "-1", "1.5"] {
            assert!(parse_interval(value).is_err(), "{value:?} was accepted");
        }
        assert!(parse_interval("five").unwrap_err().contains("not a number"));
    }
}
