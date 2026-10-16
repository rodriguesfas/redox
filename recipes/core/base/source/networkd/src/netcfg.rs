//! The network stack as a file tree.
//!
//! Everything the daemon knows about the network comes from `/scheme/netcfg`,
//! and everything it decides is written back there: the interfaces are the
//! entries of `ifaces/`, an address is written to `ifaces/<name>/addr/set`, a
//! route to `route/add`, a resolver to `resolv/nameserver`.
//!
//! The control files are opened for writing but never created: they exist
//! because the stack says they do, and a path that is not there is a path the
//! stack does not serve — creating it would silently write to the wrong place.
//! The file is committed with `sync_data`, the way the stack publishes a
//! change.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::addr::MacAddr;
use crate::plan::{Observed, Write};

/// Where the network stack is mounted.
pub const DEFAULT_ROOT: &str = "/scheme/netcfg";

/// Environment variable overriding where the stack is mounted.
pub const ENV_ROOT: &str = "NETWORKD_NETCFG";

/// What the network stack answered with, when it is not what was expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetcfgError(pub String);

impl fmt::Display for NetcfgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NetcfgError {}

impl From<std::io::Error> for NetcfgError {
    fn from(err: std::io::Error) -> Self {
        Self(err.to_string())
    }
}

/// The control files of the network stack.
#[derive(Clone, Debug)]
pub struct Netcfg {
    root: String,
}

impl Netcfg {
    pub fn new(root: impl Into<String>) -> Self {
        Self { root: root.into() }
    }

    /// The stack as the environment says it is mounted.
    pub fn from_env() -> Self {
        match std::env::var(ENV_ROOT) {
            Ok(root) if !root.is_empty() => Self::new(root),
            _ => Self::new(DEFAULT_ROOT),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// The control file a path below the root names.
    pub fn path(&self, path: &str) -> PathBuf {
        Path::new(self.root.trim_end_matches('/')).join(path)
    }

    /// Whether the stack is mounted at all.
    ///
    /// A stack that is not there yet is not a stack with no interfaces: at boot
    /// the network comes up after the daemons that use it, and a daemon that
    /// cannot tell the two apart can only fail where it should wait.
    pub fn is_mounted(&self) -> bool {
        self.path("ifaces").is_dir()
    }

    /// The interfaces the stack serves.
    ///
    /// The name of an interface is the name of its directory; its hardware
    /// address is what the stack reports for it, and a stack that has no
    /// address to report is a stack with no interface to configure.
    pub fn interfaces(&self) -> Result<Vec<Observed>, NetcfgError> {
        let dir = self.path("ifaces");
        let entries = fs::read_dir(&dir).map_err(|err| {
            NetcfgError(format!(
                "cannot read the interfaces in {}: {err}",
                dir.display()
            ))
        })?;

        let mut names = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|err| NetcfgError(format!("cannot read {}: {err}", dir.display())))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                names.push(name);
            }
        }
        names.sort();

        let mut interfaces = Vec::with_capacity(names.len());
        for name in names {
            // A stack that cannot say the hardware address still has the
            // interface, and the configuration may match it by name.
            let mac = self
                .read(&format!("ifaces/{name}/mac"))
                .ok()
                .and_then(|text| text.trim().parse::<MacAddr>().ok());
            interfaces.push(Observed { name, mac });
        }

        Ok(interfaces)
    }

    /// Reads a control file.
    pub fn read(&self, path: &str) -> Result<String, NetcfgError> {
        let path = self.path(path);
        fs::read_to_string(&path)
            .map_err(|err| NetcfgError(format!("cannot read {}: {err}", path.display())))
    }

    /// Applies one write, committing it the way the stack publishes a change.
    pub fn write(&self, write: &Write) -> Result<(), NetcfgError> {
        let path = self.path(&write.path);
        self.write_path(&path, &write.value)
    }

    fn write_path(&self, path: &Path, value: &str) -> Result<(), NetcfgError> {
        let mut file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(false)
            .open(path)
            .map_err(|err| NetcfgError(format!("cannot open {}: {err}", path.display())))?;

        file.write_all(value.as_bytes())
            .map_err(|err| NetcfgError(format!("cannot write to {}: {err}", path.display())))?;
        file.sync_data()
            .map_err(|err| NetcfgError(format!("cannot commit {}: {err}", path.display())))?;

        Ok(())
    }

    /// Applies a whole plan, in order.
    ///
    /// The writes stop at the first failure: a stack that refused an address
    /// has no business being told about the routes that sit on top of it.
    pub fn apply(&self, writes: &[Write]) -> Result<Vec<String>, NetcfgError> {
        let mut applied = Vec::with_capacity(writes.len());

        for write in writes {
            self.write(write)?;
            applied.push(write.render(&self.root));
        }

        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("networkd-netcfg-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// A stack with one interface and the control files a static address
    /// needs, which is the part of `/scheme/netcfg` this crate writes to.
    fn netcfg_with_eth0(name: &str) -> (PathBuf, Netcfg) {
        let dir = scratch(name);
        for path in [
            "ifaces/eth0/mac",
            "ifaces/eth0/addr/set",
            "route/add",
            "resolv/nameserver",
        ] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "").unwrap();
        }
        fs::write(dir.join("ifaces/eth0/mac"), "52:54:00:12:34:56\n").unwrap();

        let netcfg = Netcfg::new(dir.to_string_lossy().into_owned());
        (dir, netcfg)
    }

    #[test]
    fn the_interfaces_are_the_directories_the_stack_serves() {
        let (dir, netcfg) = netcfg_with_eth0("interfaces");

        // An interface with no hardware address yet is still an interface.
        let wlan = dir.join("ifaces/wlan0");
        fs::create_dir_all(&wlan).unwrap();
        fs::write(wlan.join("mac"), "Not configured\n").unwrap();

        let interfaces = netcfg.interfaces().unwrap();
        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces[0].name, "eth0");
        assert_eq!(interfaces[0].mac.unwrap().to_string(), "52:54:00:12:34:56");
        assert_eq!(interfaces[1].name, "wlan0");
        assert_eq!(interfaces[1].mac, None);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stack_that_is_not_there_is_an_error() {
        let netcfg = Netcfg::new(scratch("missing").to_string_lossy().into_owned());
        let error = netcfg.interfaces().unwrap_err();
        assert!(error.0.contains("cannot read the interfaces"), "{error}");
    }

    #[test]
    fn a_stack_that_is_not_mounted_is_not_a_stack_without_interfaces() {
        let netcfg = Netcfg::new(scratch("unmounted").to_string_lossy().into_owned());
        assert!(!netcfg.is_mounted(), "nothing is served there yet");
        assert!(netcfg.interfaces().is_err(), "and reading it is an error");

        // An empty stack is mounted: it serves `ifaces` and has nothing in it.
        let (dir, netcfg) = netcfg_with_eth0("mounted");
        assert!(netcfg.is_mounted());
        assert_eq!(netcfg.interfaces().unwrap().len(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_lands_in_the_control_file_the_stack_serves() {
        let (dir, netcfg) = netcfg_with_eth0("write");

        let writes = vec![
            Write {
                path: "ifaces/eth0/addr/set".to_owned(),
                value: "192.168.1.50/24\n".to_owned(),
            },
            Write {
                path: "route/add".to_owned(),
                value: "default via 192.168.1.1\n".to_owned(),
            },
        ];

        let applied = netcfg.apply(&writes).unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(
            fs::read_to_string(dir.join("ifaces/eth0/addr/set")).unwrap(),
            "192.168.1.50/24\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("route/add")).unwrap(),
            "default via 192.168.1.1\n"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_control_file_is_never_created() {
        let (dir, netcfg) = netcfg_with_eth0("create");

        // `route/rm` is a control file of the stack, but this stack does not
        // serve it: the write has to fail instead of creating a file that
        // means nothing.
        let error = netcfg
            .write(&Write {
                path: "route/rm".to_owned(),
                value: "10.0.0.0/8\n".to_owned(),
            })
            .unwrap_err();

        assert!(error.0.contains("cannot open"), "{error}");
        assert!(!dir.join("route/rm").exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn applying_stops_at_the_first_failure() {
        let (dir, netcfg) = netcfg_with_eth0("stop");

        let writes = vec![
            Write {
                path: "ifaces/eth0/addr/set".to_owned(),
                value: "192.168.1.50/24\n".to_owned(),
            },
            Write {
                path: "route/rm".to_owned(),
                value: "10.0.0.0/8\n".to_owned(),
            },
            Write {
                path: "resolv/nameserver".to_owned(),
                value: "9.9.9.9\n".to_owned(),
            },
        ];

        let error = netcfg.apply(&writes).unwrap_err();
        assert!(error.0.contains("route/rm"), "{error}");
        assert_eq!(
            fs::read_to_string(dir.join("resolv/nameserver")).unwrap(),
            "",
            "the resolver was not written after the failure"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_root_of_the_stack_can_be_overridden() {
        assert_eq!(Netcfg::from_env().root(), DEFAULT_ROOT);
    }
}
