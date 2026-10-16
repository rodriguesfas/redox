//! The work of the daemon: the requests of the scheme, applied to the state.
//!
//! The scheme is only the way in; what a request means is decided here, where it
//! can be tested on the host. Every request goes through the same state the
//! watcher writes, one at a time, so no two writes of the file can disagree.

use crate::protocol::{Outcome, ProtocolError, Request};
use crate::render::render_status_document;
use crate::state::{lock, SharedResolved};

/// The resolver configuration a daemon serves.
#[derive(Clone, Debug)]
pub struct Service {
    state: SharedResolved,
}

impl Service {
    /// Serves a state, which the watcher may be watching too.
    pub fn new(state: SharedResolved) -> Self {
        Self { state }
    }

    /// The state this service writes.
    pub fn state(&self) -> &SharedResolved {
        &self.state
    }

    /// Applies one request and answers with the message a client reads.
    ///
    /// A refusal is a [`ProtocolError`], which the scheme turns into the `err`
    /// line the client sees: the reason a resolver was refused has to reach
    /// whoever asked, not be swallowed by the daemon.
    pub fn handle(&self, request: &Request) -> Result<String, ProtocolError> {
        let resolved = lock(&self.state);

        let sync = match request {
            Request::Sync => resolved.sync(false),
            Request::SetDns(addr) => resolved.set_dns(*addr, false).map(|(_, sync)| sync),
        }
        .map_err(|err| ProtocolError(err.0))?;

        Ok(Outcome::new(&sync.path, sync.changed).encode())
    }

    /// What the file `status` of the scheme says.
    ///
    /// A configuration that cannot be read is reported instead of stopping the
    /// daemon: what a reader wants is to be told what is wrong, and a daemon
    /// that cannot answer at all tells them nothing.
    pub fn status_document(&self) -> String {
        match lock(&self.state).status() {
            Ok(status) => render_status_document(&status),
            Err(err) => format!("error: {err}\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use networkd::Netcfg;

    use super::*;
    use crate::state::{shared, Resolved, Stack, NETCFG_PATH};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("resolved-service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn service(dir: &Path, resolver: Option<&str>) -> (Service, PathBuf, PathBuf) {
        let netcfg = dir.join("netcfg");
        let control = netcfg.join(NETCFG_PATH);
        fs::create_dir_all(control.parent().unwrap()).unwrap();
        fs::write(&control, resolver.unwrap_or("")).unwrap();

        let file = dir.join("resolv.conf");
        let resolved = Resolved::new(
            file.clone(),
            Netcfg::new(netcfg.to_string_lossy().into_owned()),
        );

        (Service::new(shared(resolved)), netcfg, file)
    }

    #[test]
    fn a_sync_request_writes_the_file_and_says_so() {
        let dir = scratch("sync");
        let (service, _, file) = service(&dir, Some("9.9.9.9\n"));

        let answer = service.handle(&Request::Sync).unwrap();
        assert_eq!(answer, format!("wrote {}", file.display()));
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("nameserver 9.9.9.9\n"));

        // A second one has nothing to write, and says that instead.
        let answer = service.handle(&Request::Sync).unwrap();
        assert_eq!(answer, format!("unchanged {}", file.display()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_set_dns_request_decides_both_places() {
        let dir = scratch("set");
        let (service, netcfg, file) = service(&dir, Some("8.8.8.8\n"));

        let answer = service
            .handle(&Request::SetDns("1.1.1.1".parse().unwrap()))
            .unwrap();
        assert_eq!(answer, format!("wrote {}", file.display()));
        assert_eq!(
            fs::read_to_string(netcfg.join(NETCFG_PATH)).unwrap(),
            "1.1.1.1\n"
        );
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("nameserver 1.1.1.1\n"));

        // A resolver a query could not be sent to is refused, and nothing is
        // written: the answer reaches the client as an error.
        let err = service
            .handle(&Request::SetDns("0.0.0.0".parse().unwrap()))
            .unwrap_err();
        assert!(
            err.0.contains("not an address a query can be sent to"),
            "{err}"
        );
        assert_eq!(
            fs::read_to_string(netcfg.join(NETCFG_PATH)).unwrap(),
            "1.1.1.1\n"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stack_without_a_resolver_is_refused_out_loud() {
        let dir = scratch("empty");
        let (service, _, file) = service(&dir, None);

        let err = service.handle(&Request::Sync).unwrap_err();
        assert!(err.0.contains("holds no resolver"), "{err}");
        assert!(!file.exists());

        // A stack that is not mounted at all says that instead of pretending.
        let missing = Service::new(shared(Resolved::new(
            dir.join("other.conf"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        )));
        let err = missing.handle(&Request::Sync).unwrap_err();
        assert!(err.0.contains("cannot be read"), "{err}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_report_is_the_live_view_of_the_daemon() {
        let dir = scratch("status");
        let (service, _, file) = service(&dir, Some("9.9.9.9\n"));

        let before = service.status_document();
        assert!(
            before.contains(&format!("      File: {}", file.display())),
            "{before}"
        );
        assert!(before.contains(" Resolvers: none\n"), "{before}");
        assert!(before.contains("     Stack: 9.9.9.9\n"), "{before}");
        assert!(before.contains("   In sync: no\n"), "{before}");

        service.handle(&Request::Sync).unwrap();

        let after = service.status_document();
        assert!(after.contains(" Resolvers: 9.9.9.9\n"), "{after}");
        assert!(after.contains("   In sync: yes\n"), "{after}");

        // A file with something wrong in it says so, and does not stop the
        // daemon from answering.
        fs::write(&file, "nameserver banana\n").unwrap();
        let report = service.status_document();
        assert!(
            report.contains("warning: line 1: nameserver: banana"),
            "{report}"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_watcher_and_the_requests_write_the_same_state_one_at_a_time() {
        let dir = scratch("shared");
        let (service, netcfg, file) = service(&dir, Some("9.9.9.9\n"));

        // The watcher holds the same state, and a look it takes cannot land in
        // the middle of a request: what it reads is what the request left.
        let mut watcher =
            crate::watch::Watcher::new(service.state().clone(), crate::watch::DEFAULT_INTERVAL);

        service.handle(&Request::Sync).unwrap();
        assert!(watcher.look().unwrap().is_none(), "already written");

        fs::write(netcfg.join(NETCFG_PATH), "1.1.1.1\n").unwrap();
        let sync = watcher
            .look()
            .unwrap()
            .expect("the change has to be carried");
        assert!(sync.changed);
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("nameserver 1.1.1.1\n"));

        // And the report agrees with what both of them did.
        let report = service.status_document();
        assert!(report.contains("     Stack: 1.1.1.1\n"), "{report}");
        assert!(report.contains("   In sync: yes\n"), "{report}");

        // The stack of the daemon is where the resolver lives.
        assert!(matches!(
            crate::state::lock(service.state()).stack(),
            Stack::Resolver(_)
        ));

        let _ = fs::remove_dir_all(dir);
    }
}
