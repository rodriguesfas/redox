//! The work of the daemon: the requests of the scheme, applied to the state.
//!
//! The scheme is only the way in; what a request means is decided here, where it
//! can be tested on the host. Every request goes through the same state the
//! watcher writes, one at a time, so no two applications of the configuration
//! can disagree about what the stack already has.

use crate::protocol::{Decision, Outcome, ProtocolError, Request};
use crate::render::render_status_document;
use crate::state::{lock, SharedNetworkd};

/// The network configuration a daemon serves.
#[derive(Clone, Debug)]
pub struct Service {
    state: SharedNetworkd,
}

impl Service {
    /// Serves a state, which the watcher may be watching too.
    pub fn new(state: SharedNetworkd) -> Self {
        Self { state }
    }

    /// The state this service writes.
    pub fn state(&self) -> &SharedNetworkd {
        &self.state
    }

    /// Applies one request and answers with the message a client reads.
    ///
    /// A refusal is a [`ProtocolError`], which the scheme turns into the `err`
    /// line the client sees: why a configuration could not be applied has to
    /// reach whoever asked, not be swallowed by the daemon.
    pub fn handle(&self, request: &Request) -> Result<String, ProtocolError> {
        let mut networkd = lock(&self.state);

        match request {
            Request::Apply => {
                let application = networkd.apply().map_err(|err| ProtocolError(err.0))?;
                Ok(Outcome::of(&application).encode())
            }
            Request::Dhcp { iface } => {
                let plan = networkd
                    .plan_for(iface)
                    .map_err(|err| ProtocolError(err.0))?;
                Ok(Decision::of(&plan).encode())
            }
        }
    }

    /// What the file `status` of the scheme says.
    ///
    /// A configuration or a stack that cannot be read is reported instead of
    /// stopping the daemon: what a reader wants is to be told what is wrong, and
    /// a daemon that cannot answer at all tells them nothing.
    pub fn status_document(&self) -> String {
        let networkd = lock(&self.state);

        match networkd.plan() {
            Ok(plan) => render_status_document(
                &plan,
                networkd.netcfg().root(),
                networkd.applied().is_some(),
            ),
            Err(err) => format!("error: {err}\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::netcfg::Netcfg;
    use crate::plan::Mode;
    use crate::state::{shared, Networkd};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("networkd-service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stack with one interface, and a configuration for it.
    fn service(dir: &Path, config: Option<&str>) -> (Service, PathBuf, PathBuf) {
        let netcfg = dir.join("netcfg");
        for path in [
            "ifaces/eth0/mac",
            "ifaces/eth0/addr/set",
            "route/add",
            "resolv/nameserver",
        ] {
            let file = netcfg.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "").unwrap();
        }

        let config_dir = dir.join("networkd");
        fs::create_dir_all(&config_dir).unwrap();
        if let Some(text) = config {
            fs::write(config_dir.join("10-eth0.network"), text).unwrap();
        }

        let networkd = Networkd::new(
            config_dir.clone(),
            Netcfg::new(netcfg.to_string_lossy().into_owned()),
        );

        (Service::new(shared(networkd)), netcfg, config_dir)
    }

    #[test]
    fn an_apply_request_writes_the_stack_and_says_so() {
        let dir = scratch("apply");
        let (service, netcfg, config_dir) = service(
            &dir,
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n"),
        );

        let answer = service.handle(&Request::Apply).unwrap();
        assert_eq!(
            answer,
            format!("applied\t1\t{}\t{}", netcfg.display(), config_dir.display())
        );
        assert_eq!(
            fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
            "192.168.1.50/24\n"
        );

        // A second one has nothing to write, and says that instead.
        let answer = service.handle(&Request::Apply).unwrap();
        assert_eq!(
            answer,
            format!("unchanged\t{}\t{}", netcfg.display(), config_dir.display())
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_configuration_that_cannot_be_read_is_refused_out_loud() {
        let dir = scratch("bad");
        let (service, netcfg, config_dir) = service(&dir, None);

        // A line that is not understood is a warning, not a refusal: the parser
        // is tolerant on purpose. A file that cannot be read at all is a
        // refusal, and nothing is written out of it.
        fs::write(config_dir.join("10-eth0.network"), [0xff, 0xfe]).unwrap();

        let err = service.handle(&Request::Apply).unwrap_err();
        assert!(err.0.contains("10-eth0.network"), "{err}");
        assert_eq!(
            fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
            "",
            "nothing is written from a file that was not understood"
        );

        // A stack that is not mounted at all says that instead of pretending.
        // (The configuration is a clean one here, so what fails is the stack.)
        let absent = Service::new(shared(Networkd::new(
            dir.join("clean"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        )));
        let err = absent.handle(&Request::Apply).unwrap_err();
        assert!(err.0.contains("cannot read the interfaces"), "{err}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_report_is_the_live_view_of_the_daemon() {
        let dir = scratch("status");
        let (service, netcfg, _) = service(
            &dir,
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nGateway=192.168.1.1\n"),
        );

        let before = service.status_document();
        assert!(before.contains("eth0: static"), "{before}");
        assert!(before.contains("    address 192.168.1.50/24\n"), "{before}");
        assert!(before.contains("    gateway 192.168.1.1\n"), "{before}");
        assert!(before.ends_with("applied: no\n"), "{before}");

        let application = lock(service.state()).applied().map(<[_]>::len);
        assert_eq!(application, None, "nothing was applied yet");

        service.handle(&Request::Apply).unwrap();

        let after = service.status_document();
        assert!(after.ends_with("applied: yes\n"), "{after}");

        // What is wrong with the configuration is in the report: a reader of
        // the file has no other place to see it.
        fs::write(netcfg.join("ifaces/eth0/mac"), "52:54:00:12:34:56\n").unwrap();
        let report = service.status_document();
        assert!(report.contains("[52:54:00:12:34:56]"), "{report}");

        // A stack that is not there is reported, and does not stop the daemon.
        let absent = Service::new(shared(Networkd::new(
            dir.join("networkd"),
            Netcfg::new(dir.join("absent").to_string_lossy().into_owned()),
        )));
        let report = absent.status_document();
        assert!(
            report.starts_with("error: ") && report.contains("cannot read the interfaces"),
            "{report}"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_watcher_and_the_requests_write_the_same_state_one_at_a_time() {
        let dir = scratch("shared");
        let (service, netcfg, config_dir) = service(
            &dir,
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n"),
        );

        let mut watcher =
            crate::watch::Watcher::new(service.state().clone(), crate::watch::DEFAULT_INTERVAL);

        // The watcher applies what the request would have applied: the two hold
        // the same state, so neither repeats what the other did.
        assert!(watcher.look().unwrap().is_some());
        assert!(!service
            .handle(&Request::Apply)
            .unwrap()
            .contains("applied\t"));

        // The configuration changed, and the request carries it: the watcher
        // and the request agree on what is already in the stack.
        fs::write(
            config_dir.join("10-eth0.network"),
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n",
        )
        .unwrap();

        assert!(service
            .handle(&Request::Apply)
            .unwrap()
            .contains("applied\t1\t"));
        // A control file is a value, not a file that grows: on the host it is a
        // regular file, so the value at the start is what the stack reads.
        assert!(fs::read_to_string(netcfg.join("ifaces/eth0/addr/set"))
            .unwrap()
            .starts_with("10.0.0.2/8\n"));
        assert!(watcher.look().unwrap().is_none(), "already applied");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_decision_of_one_interface_is_what_the_configuration_says() {
        let dir = scratch("decision");
        let (service, _, config_dir) = service(
            &dir,
            Some("[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n"),
        );

        let ask = |iface: &str| {
            Decision::parse(
                &service
                    .handle(&Request::Dhcp {
                        iface: iface.to_owned(),
                    })
                    .unwrap(),
            )
            .unwrap()
        };

        // The interface the configuration addresses is not left to DHCP, and
        // the file that decided is named.
        let decision = ask("eth0");
        assert!(decision.about("eth0"));
        assert_eq!(decision.mode(), Mode::Static);
        assert_eq!(
            decision.source(),
            Some(config_dir.join("10-eth0.network").as_path())
        );

        // A file that says DHCP leaves it to the server — and the answer names
        // the file, so it is not confused with the interface below.
        fs::write(
            config_dir.join("10-eth0.network"),
            "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
        )
        .unwrap();
        let decision = ask("eth0");
        assert_eq!(decision.mode(), Mode::Dhcp);
        assert_eq!(
            decision.source(),
            Some(config_dir.join("10-eth0.network").as_path())
        );

        // An interface no file matches is left to DHCP too, and no file is
        // named: the answer says which of the two situations this is.
        let decision = ask("wlan0");
        assert_eq!(decision.mode(), Mode::Dhcp);
        assert_eq!(decision.source(), None);
        assert!(!decision.about("eth0"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_question_about_a_configuration_that_cannot_be_read_is_refused() {
        let dir = scratch("decision-bad");
        let (service, _, config_dir) = service(&dir, None);
        fs::write(config_dir.join("10-eth0.network"), [0xff, 0xfe]).unwrap();

        let err = service
            .handle(&Request::Dhcp {
                iface: "eth0".to_owned(),
            })
            .unwrap_err();
        assert!(err.0.contains("10-eth0.network"), "{err}");

        let _ = fs::remove_dir_all(dir);
    }
}
