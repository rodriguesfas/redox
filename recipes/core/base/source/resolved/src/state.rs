//! The resolver of the system: where it comes from, and where it is written.
//!
//! Two places hold a resolver: the network stack, which learns one from a DHCP
//! server (or from a `*.network` file) and keeps it at
//! `/scheme/netcfg/resolv/nameserver`, and the file a program reads,
//! `/etc/resolv.conf`. The stack is the source of truth; the file is the thing
//! that has to agree with it.
//!
//! [`Resolved`] does that: it reads the resolver of the stack, keeps the file
//! the way it was written — only the `nameserver` lines are ours — and writes
//! it when, and only when, the resolvers differ. Every operation can be asked
//! to say what it would do instead of doing it, so a command that changes the
//! network can be read before it runs.

use std::path::{Path, PathBuf};

use networkd::plan::Write;
use networkd::{Ipv4Addr, Netcfg};

use crate::resolv::{self, ResolvConf};

/// Environment variable overriding the file that is managed.
pub const ENV_FILE: &str = "RESOLVED_FILE";

/// Environment variable overriding where the network stack is mounted.
pub const ENV_NETCFG: &str = networkd::ENV_ROOT;

/// The control file of the network stack that holds the resolver.
pub const NETCFG_PATH: &str = "resolv/nameserver";

/// What went wrong while reading or writing the resolver configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedError(pub String);

impl std::fmt::Display for ResolvedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ResolvedError {}

/// The resolver the network stack answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stack {
    /// The stack holds a resolver.
    Resolver(Ipv4Addr),
    /// The stack is there and holds no resolver.
    Empty,
    /// The stack cannot be read, and why.
    Unavailable(String),
}

impl Stack {
    /// The resolver, when the stack holds one.
    pub fn resolver(&self) -> Option<Ipv4Addr> {
        match self {
            Self::Resolver(addr) => Some(*addr),
            Self::Empty | Self::Unavailable(_) => None,
        }
    }

    /// Whether the stack could be read at all.
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unavailable(_))
    }
}

impl std::fmt::Display for Stack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resolver(addr) => write!(f, "{addr}"),
            Self::Empty => f.write_str("none"),
            Self::Unavailable(message) => write!(f, "not reachable: {message}"),
        }
    }
}

/// What the system looks like right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The file that is managed.
    pub file: PathBuf,
    /// The file as it was read, when it is there.
    pub text: Option<String>,
    /// What the file says.
    pub conf: ResolvConf,
    /// What is wrong with the file.
    pub warnings: Vec<String>,
    /// The resolver of the network stack.
    pub stack: Stack,
}

impl Status {
    /// Whether the file is there.
    pub fn exists(&self) -> bool {
        self.text.is_some()
    }

    /// Whether a program that reads the file gets the resolver of the stack.
    pub fn in_sync(&self) -> bool {
        self.conf.nameservers.len() == 1 && self.conf.first() == self.stack.resolver()
    }
}

/// What happened — or what would happen — to the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sync {
    /// The file.
    pub path: PathBuf,
    /// What the file has to say.
    pub conf: ResolvConf,
    /// The file as it is written.
    pub rendered: String,
    /// Whether the file differs from what it has to say.
    pub changed: bool,
}

impl Sync {
    /// Whether the write was skipped because there was nothing to write.
    pub fn is_noop(&self) -> bool {
        !self.changed
    }
}

/// The resolver configuration of a system.
#[derive(Clone, Debug)]
pub struct Resolved {
    file: PathBuf,
    netcfg: Netcfg,
}

impl Resolved {
    /// Manages `file`, with the resolver coming from `netcfg`.
    pub fn new(file: impl Into<PathBuf>, netcfg: Netcfg) -> Self {
        Self {
            file: file.into(),
            netcfg,
        }
    }

    /// Manages the file and the stack the environment names.
    pub fn from_env() -> Self {
        let file = match std::env::var(ENV_FILE) {
            Ok(file) if !file.trim().is_empty() => PathBuf::from(file),
            _ => PathBuf::from(resolv::FILE),
        };

        Self::new(file, Netcfg::from_env())
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn netcfg(&self) -> &Netcfg {
        &self.netcfg
    }

    /// The file as it was written, when it is there.
    ///
    /// A file that is not there is not an error: a system where nothing ever
    /// wrote a resolver is exactly the system this crate exists for.
    pub fn read_text(&self) -> Result<Option<String>, ResolvedError> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(ResolvedError(format!(
                "cannot read {}: {err}",
                self.file.display()
            ))),
        }
    }

    /// What the file says, and what is wrong with it.
    pub fn read(&self) -> Result<(ResolvConf, Vec<String>), ResolvedError> {
        match self.read_text()? {
            Some(text) => Ok(resolv::parse_with_diagnostics(&text)),
            None => Ok((ResolvConf::new(), Vec::new())),
        }
    }

    /// The resolver the network stack answers with.
    pub fn stack(&self) -> Stack {
        match self.netcfg.read(NETCFG_PATH) {
            Ok(text) => match text.trim() {
                "" => Stack::Empty,
                value => match value.parse::<Ipv4Addr>() {
                    Ok(addr) => Stack::Resolver(addr),
                    Err(err) => Stack::Unavailable(format!("{NETCFG_PATH}: {err}")),
                },
            },
            Err(err) => Stack::Unavailable(err.0),
        }
    }

    /// The configuration the file has to have, with `addr` as its resolver.
    ///
    /// Everything the file says that is not a resolver is kept: `resolved` owns
    /// the `nameserver` lines, and a file that is managed must not lose what
    /// was written in it on purpose.
    pub fn conf_for(&self, addr: Ipv4Addr) -> Result<ResolvConf, ResolvedError> {
        let (conf, _) = self.read()?;
        Ok(conf.with_nameservers([addr]))
    }

    /// What the system looks like now.
    pub fn status(&self) -> Result<Status, ResolvedError> {
        let text = self.read_text()?;
        let (conf, warnings) = match &text {
            Some(text) => resolv::parse_with_diagnostics(text),
            None => (ResolvConf::new(), Vec::new()),
        };

        Ok(Status {
            file: self.file.clone(),
            text,
            conf,
            warnings,
            stack: self.stack(),
        })
    }

    /// Writes the resolver of the network stack to the file, when it differs.
    pub fn sync(&self, dry_run: bool) -> Result<Sync, ResolvedError> {
        let addr = match self.stack() {
            Stack::Resolver(addr) => addr,
            Stack::Empty => {
                return Err(ResolvedError(
                    "the network stack holds no resolver; set one with `resolvectl set-dns`"
                        .to_owned(),
                ))
            }
            Stack::Unavailable(message) => {
                return Err(ResolvedError(format!(
                    "the resolver of the network stack cannot be read: {message}"
                )))
            }
        };

        self.commit(self.conf_for(addr)?, dry_run)
    }

    /// Sets the resolver of the network stack, and of the file.
    ///
    /// The address is checked the way the stack checks it, so a value the stack
    /// would refuse is refused here, before anything is written.
    pub fn set_dns(&self, addr: Ipv4Addr, dry_run: bool) -> Result<(Write, Sync), ResolvedError> {
        if !addr.is_unicast() {
            return Err(ResolvedError(format!(
                "{addr}: not an address a query can be sent to"
            )));
        }

        let write = Write {
            path: NETCFG_PATH.to_owned(),
            value: format!("{addr}\n"),
        };

        if !dry_run {
            self.netcfg.write(&write).map_err(|err| {
                ResolvedError(format!(
                    "cannot set the resolver of the network stack: {}",
                    err.0
                ))
            })?;
        }

        let sync = self.commit(self.conf_for(addr)?, dry_run)?;
        Ok((write, sync))
    }

    /// Writes the file, if what it says is not what it has to say.
    fn commit(&self, conf: ResolvConf, dry_run: bool) -> Result<Sync, ResolvedError> {
        let rendered = conf.render();
        let (current, _) = self.read()?;
        let changed = !current.same_resolvers(&conf);

        if changed && !dry_run {
            std::fs::write(&self.file, rendered.as_bytes()).map_err(|err| {
                ResolvedError(format!("cannot write {}: {err}", self.file.display()))
            })?;
        }

        Ok(Sync {
            path: self.file.clone(),
            conf,
            rendered,
            changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("resolved-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stack with the control file that holds the resolver.
    fn stack_with(dir: &Path, resolver: Option<&str>) -> Netcfg {
        let control = dir.join("netcfg").join(NETCFG_PATH);
        std::fs::create_dir_all(control.parent().unwrap()).unwrap();
        std::fs::write(&control, resolver.unwrap_or("")).unwrap();

        Netcfg::new(dir.join("netcfg").to_string_lossy().into_owned())
    }

    fn resolved(dir: &Path, file: &str, resolver: Option<&str>) -> Resolved {
        let netcfg = stack_with(dir, resolver);
        Resolved::new(dir.join(file), netcfg)
    }

    #[test]
    fn the_stack_says_what_the_resolver_is() {
        let dir = scratch("stack");
        assert_eq!(
            resolved(&dir, "resolv.conf", Some("9.9.9.9\n")).stack(),
            Stack::Resolver("9.9.9.9".parse().unwrap())
        );

        assert_eq!(resolved(&dir, "resolv.conf", None).stack(), Stack::Empty);

        // A stack that is not running is not a stack without a resolver.
        let missing = Resolved::new(
            dir.join("resolv.conf"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        );
        let stack = missing.stack();
        assert!(!stack.is_available(), "{stack}");
        assert!(stack.to_string().starts_with("not reachable: "), "{stack}");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_there_is_not_an_error() {
        let dir = scratch("missing");
        let resolved = resolved(&dir, "resolv.conf", Some("9.9.9.9"));

        assert_eq!(resolved.read_text().unwrap(), None);
        let (conf, warnings) = resolved.read().unwrap();
        assert!(conf.is_empty());
        assert!(warnings.is_empty());

        let status = resolved.status().unwrap();
        assert!(!status.exists());
        assert!(!status.in_sync(), "no file, no resolver to read");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_resolver_of_the_stack_reaches_the_file() {
        let dir = scratch("sync");
        let resolved = resolved(&dir, "resolv.conf", Some("1.1.1.1\n"));

        // Nothing is written until asked, and a dry run says what would be.
        let sync = resolved.sync(true).unwrap();
        assert!(sync.changed);
        assert!(
            !dir.join("resolv.conf").exists(),
            "a dry run writes nothing"
        );

        let sync = resolved.sync(false).unwrap();
        assert!(sync.changed);
        assert_eq!(
            std::fs::read_to_string(dir.join("resolv.conf")).unwrap(),
            sync.rendered
        );

        let status = resolved.status().unwrap();
        assert!(status.exists());
        assert!(status.in_sync());
        assert_eq!(status.conf.first().unwrap().to_string(), "1.1.1.1");

        // And a second time there is nothing left to do.
        let sync = resolved.sync(false).unwrap();
        assert!(sync.is_noop());
        assert!(!sync.changed);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_the_file_says_beyond_the_resolver_is_kept() {
        let dir = scratch("keep");
        let file = dir.join("resolv.conf");
        std::fs::write(
            &file,
            "# written by hand\nnameserver 8.8.8.8\nsearch example.org local\noptions ndots:1\nsortlist 10.0.0.0/255.0.0.0\n",
        )
        .unwrap();

        let resolved = resolved(&dir, "resolv.conf", Some("9.9.9.9\n"));
        let sync = resolved.sync(false).unwrap();
        assert!(sync.changed);

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("nameserver 9.9.9.9\n"), "{text}");
        assert!(!text.contains("8.8.8.8"), "{text}");
        assert!(text.contains("search example.org local\n"), "{text}");
        assert!(text.contains("options ndots:1\n"), "{text}");
        assert!(
            text.contains("sortlist 10.0.0.0/255.0.0.0\n"),
            "{text}: a line that is not understood is not a line to drop"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_already_agrees_is_not_rewritten() {
        let dir = scratch("agree");
        let file = dir.join("resolv.conf");
        // A file someone wrote by hand, saying the same thing: rewriting it
        // would only lose the way it was written.
        std::fs::write(&file, "# my file\nnameserver 9.9.9.9\n").unwrap();

        let resolved = resolved(&dir, "resolv.conf", Some("9.9.9.9\n"));
        let sync = resolved.sync(false).unwrap();
        assert!(sync.is_noop());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# my file\nnameserver 9.9.9.9\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_resolver_can_be_set_in_the_stack_and_in_the_file() {
        let dir = scratch("set");
        let resolved = resolved(&dir, "resolv.conf", Some("8.8.8.8\n"));

        let (write, sync) = resolved.set_dns("1.1.1.1".parse().unwrap(), false).unwrap();
        assert_eq!(write.path, NETCFG_PATH);
        assert_eq!(write.value, "1.1.1.1\n");
        assert!(sync.changed);
        assert_eq!(
            std::fs::read_to_string(dir.join("netcfg").join(NETCFG_PATH)).unwrap(),
            "1.1.1.1\n"
        );
        assert!(std::fs::read_to_string(dir.join("resolv.conf"))
            .unwrap()
            .contains("nameserver 1.1.1.1\n"));

        // A dry run changes neither of them.
        let (_, sync) = resolved.set_dns("9.9.9.9".parse().unwrap(), true).unwrap();
        assert!(sync.changed);
        assert_eq!(
            std::fs::read_to_string(dir.join("netcfg").join(NETCFG_PATH)).unwrap(),
            "1.1.1.1\n"
        );
        assert!(std::fs::read_to_string(dir.join("resolv.conf"))
            .unwrap()
            .contains("nameserver 1.1.1.1\n"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_resolver_the_stack_would_refuse_is_refused_here() {
        let dir = scratch("refuse");
        let resolved = resolved(&dir, "resolv.conf", Some("8.8.8.8\n"));

        for text in ["0.0.0.0", "255.255.255.255", "224.0.0.1"] {
            let err = resolved.set_dns(text.parse().unwrap(), false).unwrap_err();
            assert!(
                err.0.contains("not an address a query can be sent to"),
                "{text}: {err}"
            );
        }

        // Nothing was written, and nothing was changed in the stack.
        assert_eq!(
            std::fs::read_to_string(dir.join("netcfg").join(NETCFG_PATH)).unwrap(),
            "8.8.8.8\n"
        );
        assert!(!dir.join("resolv.conf").exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stack_without_a_resolver_cannot_be_written_to_the_file() {
        let dir = scratch("empty");
        let resolved = resolved(&dir, "resolv.conf", None);

        let err = resolved.sync(false).unwrap_err();
        assert!(err.0.contains("holds no resolver"), "{err}");
        assert!(!dir.join("resolv.conf").exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stack_that_is_not_running_is_an_error_and_not_an_empty_resolver() {
        let dir = scratch("down");
        let resolved = Resolved::new(
            dir.join("resolv.conf"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        );

        let err = resolved.sync(false).unwrap_err();
        assert!(err.0.contains("cannot be read"), "{err}");

        let status = resolved.status().unwrap();
        assert!(!status.stack.is_available());
        assert!(!status.in_sync());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_environment_names_the_file_and_the_stack() {
        // Nothing is set in a normal run: the defaults are the ones the system
        // uses. A test runner that sets the variables gets the variables.
        let expected_file = match std::env::var(ENV_FILE) {
            Ok(file) if !file.trim().is_empty() => file,
            _ => resolv::FILE.to_owned(),
        };
        let expected_root = match std::env::var(ENV_NETCFG) {
            Ok(root) if !root.trim().is_empty() => root,
            _ => networkd::DEFAULT_ROOT.to_owned(),
        };

        assert_eq!(Resolved::from_env().file(), Path::new(&expected_file));
        assert_eq!(Resolved::from_env().netcfg().root(), expected_root.as_str());
    }
}
