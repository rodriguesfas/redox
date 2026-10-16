//! Keeping the file in step with the stack while the system runs.
//!
//! The file has to be written when the resolver changes, and the resolver
//! changes without asking anyone: DHCP answers when it answers, and a
//! configuration is applied when someone applies it. The daemon therefore looks
//! at the stack from time to time — and a look that finds nothing to do costs
//! one read of one control file, which is what makes looking often the cheap
//! and boring answer.
//!
//! Everything here is pure and runs on the host: the daemon is only the thread
//! that calls [`Watcher::look`] in a loop.

use std::time::Duration;

use crate::state::{lock, ResolvedError, SharedResolved, Stack, Sync};

/// Environment variable that says how often the stack is looked at.
pub const ENV_INTERVAL: &str = "RESOLVED_INTERVAL";

/// How often the stack is looked at when the environment says nothing.
///
/// Five seconds is a compromise a resolver can live with: long enough that an
/// idle system is not woken for nothing, short enough that a machine which just
/// got its address can resolve a name by the time a program asks.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);

/// Reads the interval a value asks for.
///
/// The value is a number of seconds. `0` turns the watching off, which is what
/// a system whose resolver is decided once wants. Anything that is not a number
/// is an error for the caller to report: a typo in the environment must not
/// silently turn the file into a file that never changes.
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

/// Looks at the stack, and writes the file when the stack says something else.
pub struct Watcher {
    state: SharedResolved,
    interval: Duration,
    looks: u64,
}

impl Watcher {
    /// Watches a configuration, looking every `interval`.
    pub fn new(state: SharedResolved, interval: Duration) -> Self {
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

    /// How many times the stack was looked at.
    pub fn looks(&self) -> u64 {
        self.looks
    }

    /// One look at the stack.
    ///
    /// A stack that is not there, or that holds no resolver yet, is not a
    /// failure: the network comes up after this daemon does, and a resolver
    /// arrives when a lease arrives. In that case nothing is written and the
    /// next look tries again, which is the whole point of watching. What is
    /// answered is the write when it happened, so a caller can say so.
    pub fn look(&mut self) -> Result<Option<Sync>, ResolvedError> {
        self.looks += 1;

        let resolved = lock(&self.state);
        match resolved.stack() {
            Stack::Resolver(_) => {}
            Stack::Empty | Stack::Unavailable(_) => return Ok(None),
        }

        let sync = resolved.sync(false)?;
        Ok(if sync.changed { Some(sync) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use networkd::Netcfg;

    use super::*;
    use crate::state::{shared, Resolved, NETCFG_PATH};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("resolved-watch-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stack that holds `resolver`, and a file that is watched.
    fn watched(dir: &Path, resolver: Option<&str>) -> (SharedResolved, PathBuf, PathBuf) {
        let netcfg = dir.join("netcfg");
        let control = netcfg.join(NETCFG_PATH);
        fs::create_dir_all(control.parent().unwrap()).unwrap();
        fs::write(&control, resolver.unwrap_or("")).unwrap();

        let file = dir.join("resolv.conf");
        let state = shared(Resolved::new(
            file.clone(),
            Netcfg::new(netcfg.to_string_lossy().into_owned()),
        ));

        (state, netcfg, file)
    }

    #[test]
    fn a_look_writes_the_file_when_the_stack_says_something_new() {
        let dir = scratch("look");
        let (state, netcfg, file) = watched(&dir, Some("1.1.1.1\n"));
        let mut watcher = Watcher::new(state, DEFAULT_INTERVAL);

        assert_eq!(watcher.interval(), DEFAULT_INTERVAL);
        assert_eq!(watcher.looks(), 0);

        // The stack holds a resolver and the file is not there: it is written.
        let sync = watcher.look().unwrap().expect("the file has to be written");
        assert!(sync.changed);
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("nameserver 1.1.1.1\n"));
        assert_eq!(watcher.looks(), 1);

        // The file agrees: a look finds nothing to do.
        assert!(watcher.look().unwrap().is_none());
        assert_eq!(watcher.looks(), 2);

        // The resolver changed behind our back, as a lease does: the next look
        // is what carries it into the file.
        fs::write(netcfg.join(NETCFG_PATH), "9.9.9.9\n").unwrap();
        let sync = watcher
            .look()
            .unwrap()
            .expect("the change has to be carried");
        assert!(sync.changed);
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("nameserver 9.9.9.9\n"));

        // A file that someone corrected by hand is written back.
        fs::write(&file, "nameserver 8.8.8.8\n").unwrap();
        assert!(watcher.look().unwrap().is_some());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_look_that_finds_no_stack_is_not_a_failure() {
        let dir = scratch("no-stack");

        // A stack that is there and holds no resolver: at boot, before a lease.
        let (state, netcfg, file) = watched(&dir, None);
        let mut watcher = Watcher::new(state, DEFAULT_INTERVAL);
        assert!(watcher.look().unwrap().is_none());
        assert!(!file.exists(), "nothing to write, and nothing written");

        // A lease arrives: the same watcher carries it, without being told.
        fs::write(netcfg.join(NETCFG_PATH), "1.1.1.1\n").unwrap();
        assert!(watcher.look().unwrap().is_some());

        // A stack that is not mounted at all: no netstack yet.
        let missing = shared(Resolved::new(
            dir.join("other.conf"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        ));
        let mut watcher = Watcher::new(missing, DEFAULT_INTERVAL);
        assert!(watcher.look().unwrap().is_none());
        assert!(!dir.join("other.conf").exists());

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
