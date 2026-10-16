//! `resolved`: the resolver configuration daemon.
//!
//! It owns `/etc/resolv.conf`. It writes it at boot, writes it again when the
//! network stack says something else — looking every few seconds, and at once
//! when a request asks — and serves the same state at `/scheme/resolved`, so
//! that a tool or a daemon that learns a new resolver asks for a write instead
//! of writing the file itself.
//!
//! `init` starts it as a scheme service, after the network stack it reads. A
//! network that is not up yet is not a failure: the daemon starts, and the
//! resolver appears in the file when the lease does.

#[cfg(target_os = "redox")]
mod daemon {
    use std::rc::Rc;
    use std::thread;
    use std::time::Duration;

    use daemon::SchemeDaemon;
    use redox_scheme::Socket;
    use resolved::scheme::{ResolvedResource, ResolvedScheme, ResolvedSchemeData, Root};
    use resolved::service::Service;
    use resolved::state::{shared, Resolved};
    use resolved::watch::{parse_interval, Watcher, DEFAULT_INTERVAL, ENV_INTERVAL};
    use scheme_utils::Blocking;

    /// How often the stack is looked at, as the environment asks.
    ///
    /// A value that is not a number of seconds is reported and the default is
    /// used: a typo in the environment must not leave the file never written.
    fn interval() -> Option<Duration> {
        match std::env::var(ENV_INTERVAL) {
            Ok(value) => match parse_interval(&value) {
                Ok(interval) => interval,
                Err(err) => {
                    eprintln!(
                        "resolved: {err}; looking every {}s",
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
            Ok(Some(sync)) => println!("resolved: wrote {}", sync.path.display()),
            Ok(None) => {}
            Err(err) => eprintln!("resolved: cannot write the resolver: {err}"),
        }
    }

    pub fn main() -> ! {
        SchemeDaemon::new(|daemon| {
            let socket = Rc::new(Socket::create().expect("resolved: failed to create the scheme"));

            let state = shared(Resolved::from_env());
            let service = Service::new(state.clone());

            let interval = interval();
            let mut watcher = Watcher::new(state, interval.unwrap_or(DEFAULT_INTERVAL));

            // The first look happens before the service is announced, so that a
            // resolver that is already known is written before anything can ask
            // for it. A network that is not up yet is only a reason to say so
            // and to look again.
            match watcher.look() {
                Ok(Some(sync)) => println!("resolved: wrote {}", sync.path.display()),
                Ok(None) => println!("resolved: no resolver to write yet"),
                Err(err) => eprintln!("resolved: cannot write the resolver: {err}"),
            }

            if let Some(interval) = interval {
                thread::spawn(move || loop {
                    thread::sleep(interval);
                    look(&mut watcher);
                });
            }

            let mut scheme = ResolvedScheme::new(
                "resolved".to_owned(),
                ResolvedSchemeData::new(service),
                ResolvedResource::Root(Root),
            );
            let handler = Blocking::new(&*socket, 16);

            let _ = daemon.ready_sync_scheme(&socket, &mut scheme);

            handler
                .process_requests_blocking(scheme)
                .expect("resolved: failed to serve the resolver configuration");
        })
    }
}

#[cfg(target_os = "redox")]
fn main() {
    daemon::main();
}

/// Nothing to manage where there is no Redox boot to write for: the daemon is
/// part of the boot, and the crate still has to build for the host so that its
/// logic can be tested there.
#[cfg(not(target_os = "redox"))]
fn main() {
    eprintln!("resolved: the resolver daemon only runs on Redox");
    std::process::exit(1);
}
