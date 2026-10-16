//! What the network manager decides, and what it has already decided.
//!
//! The configuration is a directory of `*.network` files, the network is the
//! file tree the stack serves, and a [`Networkd`] is the two of them held
//! together: it reads the configuration, resolves it against the interfaces
//! that are there, and writes the result to the stack.
//!
//! A plan is applied **once**. The writes of a plan are changes, not a report:
//! writing the same address at every look would push the same change into the
//! stack forever, for a network that has not moved. So the last applied plan is
//! remembered, and a plan that says the same thing is answered with
//! "unchanged" instead of being written again.
//!
//! Nothing here sleeps, serves, or talks to a scheme: it is the logic of the
//! daemon, and it runs — and is tested — on the host.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::cli::ENV_CONFIG;
use crate::config::{Config, ConfigError, CONFIG_DIR};
use crate::netcfg::{Netcfg, NetcfgError};
use crate::plan::{Plan, Write};

/// Returned when the configuration or the stack cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkdError(pub String);

impl std::fmt::Display for NetworkdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NetworkdError {}

impl From<NetcfgError> for NetworkdError {
    fn from(err: NetcfgError) -> Self {
        Self(err.0)
    }
}

impl From<ConfigError> for NetworkdError {
    fn from(err: ConfigError) -> Self {
        Self(err.0)
    }
}

/// What applying the configuration did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// The root of the stack that was read and written.
    pub netcfg: String,
    /// The directory the configuration was read from.
    pub config: PathBuf,
    /// The writes, as they were rendered, in the order they were applied. Empty
    /// when the stack was left alone.
    pub writes: Vec<String>,
    /// Whether the stack was written to.
    pub changed: bool,
}

impl Application {
    /// Whether this is about the same stack and the same configuration.
    ///
    /// A daemon that answered about another file tree is a daemon for another
    /// system: a client that named its own cannot take its answer for its own.
    pub fn describes(&self, netcfg: &Path, config: &Path) -> bool {
        Path::new(&self.netcfg) == netcfg && self.config == config
    }
}

/// The configuration of a system, and the stack it is written to.
#[derive(Clone, Debug)]
pub struct Networkd {
    netcfg: Netcfg,
    config_dir: PathBuf,
    applied: Option<Vec<Write>>,
}

impl Networkd {
    /// Manages a configuration directory against a network stack.
    pub fn new(config_dir: impl Into<PathBuf>, netcfg: Netcfg) -> Self {
        Self {
            netcfg,
            config_dir: config_dir.into(),
            applied: None,
        }
    }

    /// Manages what the environment selects.
    pub fn from_env() -> Self {
        let config_dir = match std::env::var(ENV_CONFIG) {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(CONFIG_DIR),
        };

        Self::new(config_dir, Netcfg::from_env())
    }

    /// The directory the `*.network` files are read from.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// The stack that is read and written.
    pub fn netcfg(&self) -> &Netcfg {
        &self.netcfg
    }

    /// The writes of the plan that was applied last, if any.
    pub fn applied(&self) -> Option<&[Write]> {
        self.applied.as_deref()
    }

    /// The configuration of the system.
    ///
    /// A directory that cannot be read is an error, but a directory that is not
    /// there is not: a system with no configuration is a system where every
    /// interface is left to DHCP, which is how it comes up today.
    pub fn config(&self) -> Result<Config, NetworkdError> {
        Ok(Config::load(&self.config_dir)?)
    }

    /// What the configuration means for the interfaces of the system.
    pub fn plan(&self) -> Result<Plan, NetworkdError> {
        let config = self.config()?;
        let observed = self.netcfg.interfaces()?;

        Ok(Plan::build(&config, &observed))
    }

    /// Makes the stack look like the configuration.
    ///
    /// The writes stop at the first refusal of the stack, and what was written
    /// is remembered only when all of them landed: a half applied plan is a
    /// plan that has to be applied again, not one that is done.
    pub fn apply(&mut self) -> Result<Application, NetworkdError> {
        let writes = self.plan()?.writes();
        let unchanged = || Application {
            netcfg: self.netcfg.root().to_owned(),
            config: self.config_dir.clone(),
            writes: Vec::new(),
            changed: false,
        };

        // A plan with nothing to write is a plan that is already true: the
        // address of a DHCP interface arrives with the lease, and the routes
        // and the resolver arrive with it.
        if writes.is_empty() {
            self.applied = Some(writes);
            return Ok(unchanged());
        }

        if self.applied.as_deref() == Some(writes.as_slice()) {
            return Ok(unchanged());
        }

        let applied = self.netcfg.apply(&writes)?;
        self.applied = Some(writes);

        Ok(Application {
            netcfg: self.netcfg.root().to_owned(),
            config: self.config_dir.clone(),
            writes: applied,
            changed: true,
        })
    }
}

/// The state, shared between whoever writes the stack.
///
/// The daemon writes the stack for a request, and its watcher writes it because
/// the configuration or the interfaces changed; one writer at a time means a
/// plan read before a write cannot be applied after it.
pub type SharedNetworkd = Arc<Mutex<Networkd>>;

/// Puts a manager behind a lock, to be shared.
pub fn shared(networkd: Networkd) -> SharedNetworkd {
    Arc::new(Mutex::new(networkd))
}

/// Locks the shared state.
///
/// A panic in another thread while the lock was held is not a reason to refuse
/// to configure the network: what is behind it is a directory and a mount
/// point, and the only cost of a poisoned lock is that the last plan applied
/// may be forgotten, which means one plan is applied twice — the only
/// direction in which that is safe.
pub fn lock(state: &SharedNetworkd) -> MutexGuard<'_, Networkd> {
    state.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::plan::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("networkd-state-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stack with one interface, and a configuration directory.
    fn system(name: &str, config: Option<&str>) -> (PathBuf, PathBuf, Networkd) {
        let dir = scratch(name);

        let netcfg_dir = dir.join("netcfg");
        for path in [
            "ifaces/eth0/mac",
            "ifaces/eth0/addr/set",
            "route/add",
            "resolv/nameserver",
        ] {
            let file = netcfg_dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "").unwrap();
        }
        fs::write(netcfg_dir.join("ifaces/eth0/mac"), "52:54:00:12:34:56\n").unwrap();

        let config_dir = dir.join("networkd");
        fs::create_dir_all(&config_dir).unwrap();
        if let Some(text) = config {
            fs::write(config_dir.join("10-eth0.network"), text).unwrap();
        }

        let networkd = Networkd::new(
            config_dir.clone(),
            Netcfg::new(netcfg_dir.to_string_lossy().into_owned()),
        );

        (dir, netcfg_dir, networkd)
    }

    #[test]
    fn a_static_configuration_is_written_to_the_stack_once() {
        let (dir, netcfg_dir, mut networkd) = system(
            "static",
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nGateway=192.168.1.1\nDNS=9.9.9.9\n"),
        );

        let application = networkd.apply().unwrap();
        assert!(application.changed);
        assert_eq!(application.writes.len(), 3);
        assert!(application.describes(&netcfg_dir, networkd.config_dir()));
        assert_eq!(
            fs::read_to_string(netcfg_dir.join("ifaces/eth0/addr/set")).unwrap(),
            "192.168.1.50/24\n"
        );
        assert_eq!(
            fs::read_to_string(netcfg_dir.join("route/add")).unwrap(),
            "default via 192.168.1.1\n"
        );
        assert_eq!(
            fs::read_to_string(netcfg_dir.join("resolv/nameserver")).unwrap(),
            "9.9.9.9\n"
        );

        // The same plan again is not applied again: the writes are changes, and
        // this network has not moved.
        let application = networkd.apply().unwrap();
        assert!(!application.changed);
        assert!(application.writes.is_empty());
        assert_eq!(networkd.applied().unwrap().len(), 3);

        // A configuration that changed is a plan that has to be applied. A
        // control file is a value, not a file that grows: on the host it is a
        // regular file, so a shorter value leaves the tail of the previous one,
        // and what the stack reads is the value at the start.
        fs::write(
            networkd.config_dir().join("10-eth0.network"),
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n",
        )
        .unwrap();
        let application = networkd.apply().unwrap();
        assert!(application.changed);
        assert!(fs::read_to_string(netcfg_dir.join("ifaces/eth0/addr/set"))
            .unwrap()
            .starts_with("10.0.0.2/8\n"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dhcp_interface_has_nothing_to_write() {
        let (dir, netcfg_dir, mut networkd) =
            system("dhcp", Some("[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n"));

        // The address, the route and the resolver come from the lease: the
        // configuration is a decision about who decides, and there is nothing
        // to write for it.
        let application = networkd.apply().unwrap();
        assert!(!application.changed);
        assert!(application.writes.is_empty());
        assert_eq!(
            fs::read_to_string(netcfg_dir.join("ifaces/eth0/addr/set")).unwrap(),
            ""
        );

        // And a system with no configuration is the same system: DHPC by
        // default, which is what the boot does today.
        let (dir_empty, netcfg_empty, mut networkd) = system("none", None);
        assert!(networkd.config().unwrap().is_empty());
        assert!(!networkd.apply().unwrap().changed);
        assert_eq!(
            fs::read_to_string(netcfg_empty.join("route/add")).unwrap(),
            ""
        );

        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir_empty);
    }

    #[test]
    fn a_stack_that_is_not_mounted_is_an_error_and_not_an_empty_network() {
        let dir = scratch("no-stack");
        let networkd = Networkd::new(
            dir.join("networkd"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        );

        let error = networkd.plan().unwrap_err();
        assert!(error.0.contains("cannot read the interfaces"), "{error}");
    }

    #[test]
    fn a_configuration_that_cannot_be_read_is_an_error() {
        let (dir, _, mut networkd) = system("bad-config", None);

        // A line that is not understood is a warning, not an error: the parser
        // is tolerant on purpose. A file that cannot be read at all is an error,
        // and nothing is written out of a configuration nobody could read.
        fs::write(
            networkd.config_dir().join("10-eth0.network"),
            [0xff, 0xfe, 0x00],
        )
        .unwrap();

        let error = networkd.config().unwrap_err();
        assert!(error.0.contains("10-eth0.network"), "{error}");
        assert!(networkd.apply().unwrap_err().0.contains("10-eth0.network"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_the_stack_refuses_is_not_remembered() {
        let (dir, netcfg_dir, mut networkd) = system(
            "refused",
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nGateway=192.168.1.1\n"),
        );

        // The route is a control file of the stack this one does not serve: the
        // address is written, the route is refused, and the plan is not done.
        fs::remove_file(netcfg_dir.join("route/add")).unwrap();
        let error = networkd.apply().unwrap_err();
        assert!(error.0.contains("route/add"), "{error}");
        assert_eq!(
            networkd.applied(),
            None,
            "a half applied plan is not a plan that is done"
        );

        // The stack serves it again, and the same plan finishes.
        fs::write(netcfg_dir.join("route/add"), "").unwrap();
        assert!(networkd.apply().unwrap().changed);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_state_is_shared_one_writer_at_a_time() {
        let (dir, _, networkd) = system(
            "shared",
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n"),
        );

        let state = shared(networkd);
        assert!(lock(&state).apply().unwrap().changed);
        assert!(!lock(&state).apply().unwrap().changed);

        // A poisoned lock still serves: the file tree and the mount point are
        // not made wrong by a panic somewhere else.
        let clone = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock().unwrap();
            panic!("while holding the lock");
        })
        .join();
        assert!(lock(&state).plan().is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_keeps_the_control_file_it_names() {
        let write = Write {
            path: "ifaces/eth0/addr/set".to_owned(),
            value: "10.0.0.2/8\n".to_owned(),
        };
        assert_eq!(
            write.render("/scheme/netcfg"),
            "write /scheme/netcfg/ifaces/eth0/addr/set 10.0.0.2/8"
        );
    }
}
