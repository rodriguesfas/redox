//! `networkd`: the network configuration daemon.
//!
//! It owns the configuration of the interfaces. It applies it at boot, applies
//! it again when the configuration or the interfaces change — looking every few
//! seconds, and at once when a request asks — and serves the same state at
//! `/scheme/networkd`, so that a tool or a daemon that changes the network asks
//! for it instead of writing the stack itself.
//!
//! `init` starts it as a scheme service, before the network it configures. The
//! stack that is not up yet is not a failure: the daemon starts, and the
//! configuration is applied when the stack appears.

#[cfg(target_os = "redox")]
mod daemon {
    use std::rc::Rc;
    use std::thread;
    use std::time::Duration;

    use daemon::SchemeDaemon;
    use networkd::scheme::{NetworkdResource, NetworkdScheme, NetworkdSchemeData, Root};
    use networkd::service::Service;
    use networkd::state::{shared, Networkd};
    use networkd::watch::{parse_interval, Watcher, DEFAULT_INTERVAL, ENV_INTERVAL};
    use redox_scheme::Socket;
    use scheme_utils::Blocking;

    /// How often the configuration is looked at, as the environment asks.
    ///
    /// A value that is not a number of seconds is reported and the default is
    /// used: a typo in the environment must not leave the configuration never
    /// applied.
    fn interval() -> Option<Duration> {
        match std::env::var(ENV_INTERVAL) {
            Ok(value) => match parse_interval(&value) {
                Ok(interval) => interval,
                Err(err) => {
                    eprintln!(
                        "networkd: {err}; looking every {}s",
                        DEFAULT_INTERVAL.as_secs()
                    );
                    Some(DEFAULT_INTERVAL)
                }
            },
            Err(_) => Some(DEFAULT_INTERVAL),
        }
    }

    /// One look, reported the way a daemon reports to the log.
    fn look(watcher: &mut Watcher) {
        match watcher.look() {
            Ok(Some(application)) => println!(
                "networkd: applied {} writes to {}",
                application.writes.len(),
                application.netcfg
            ),
            Ok(None) => {}
            Err(err) => eprintln!("networkd: cannot apply the configuration: {err}"),
        }
    }

    pub fn main() -> ! {
        SchemeDaemon::new(|daemon| {
            let socket = Rc::new(Socket::create().expect("networkd: failed to create the scheme"));

            let state = shared(Networkd::from_env());
            let service = Service::new(state.clone());

            let interval = interval();
            let mut watcher = Watcher::new(state, interval.unwrap_or(DEFAULT_INTERVAL));

            // The first look happens before the service is announced, so that a
            // configuration that can be applied is applied before anything can
            // ask for it. A stack that is not up yet is only a reason to say so
            // and to look again.
            match watcher.look() {
                Ok(Some(application)) => println!(
                    "networkd: applied {} writes to {}",
                    application.writes.len(),
                    application.netcfg
                ),
                Ok(None) => println!("networkd: nothing to apply yet"),
                Err(err) => eprintln!("networkd: cannot apply the configuration: {err}"),
            }

            if let Some(interval) = interval {
                thread::spawn(move || loop {
                    thread::sleep(interval);
                    look(&mut watcher);
                });
            }

            let mut scheme = NetworkdScheme::new(
                "networkd".to_owned(),
                NetworkdSchemeData::new(service),
                NetworkdResource::Root(Root),
            );
            let handler = Blocking::new(&*socket, 16);

            let _ = daemon.ready_sync_scheme(&socket, &mut scheme);

            handler
                .process_requests_blocking(scheme)
                .expect("networkd: failed to serve the network configuration");
        })
    }
}

#[cfg(target_os = "redox")]
fn main() {
    daemon::main();
}

/// Nothing to manage where there is no Redox boot to configure: the daemon is
/// part of the boot, and the crate still has to build for the host so that its
/// logic can be tested there.
#[cfg(not(target_os = "redox"))]
fn main() {
    eprintln!("networkd: the network daemon only runs on Redox");
    std::process::exit(1);
}
