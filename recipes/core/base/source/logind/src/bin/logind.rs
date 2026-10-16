//! `logind`: the login manager daemon.
//!
//! It owns the session state of the boot and serves it at `/scheme/logind`,
//! keeping the login records (`/var/run/utmp`) in step with it, so that the live
//! view and the file tell the same story. `init` starts it as a scheme service,
//! which is also what makes the scheme available to the processes of the boot
//! namespace — `getty` and `login` among them.
//!
//! Unlike `logd`, the daemon does not retreat into an empty namespace: keeping
//! the login records up to date needs the file schemes at every request.

#[cfg(target_os = "redox")]
mod daemon {
    use std::cell::RefCell;
    use std::rc::Rc;

    use daemon::SchemeDaemon;
    use logind::registry::Registry;
    use logind::scheme::{LogindResource, LogindScheme, LogindSchemeData, Root};
    use logind::utmp::UtmpFile;
    use redox_scheme::Socket;
    use scheme_utils::Blocking;

    /// Environment variable overriding the login record file.
    pub const ENV_UTMP: &str = "LOGIND_UTMP";

    /// The login record file, as the rest of the system knows it.
    pub const DEFAULT_UTMP: &str = "/var/run/utmp";

    /// Environment variable overriding the identifier of this boot.
    pub const ENV_BOOT_ID: &str = "LOGIND_BOOT_ID";

    /// The identifier of this boot.
    ///
    /// The journal's identifier is preferred when the environment carries it,
    /// so that `loginctl` and `journalctl` describe the same boot; a boot that
    /// has to make up its own gets one derived from the time and the pid.
    fn boot_id(now: i64) -> String {
        for name in [ENV_BOOT_ID, journald::writer::ENV_BOOT_ID] {
            if let Ok(value) = std::env::var(name) {
                if journald::store::is_valid_boot_id(&value) {
                    return value;
                }
            }
        }

        journald::writer::generate_boot_id(now, u64::from(std::process::id()))
    }

    pub fn main() -> ! {
        SchemeDaemon::new(|daemon| {
            let socket = Rc::new(Socket::create().expect("logind: failed to create the scheme"));

            let path = std::env::var(ENV_UTMP).unwrap_or_else(|_| DEFAULT_UTMP.to_owned());
            let now = journald::time::unix_now();
            let boot_id = boot_id(now);

            let utmp = UtmpFile::new(&path);
            let registry = match Registry::boot(utmp, boot_id.clone(), now) {
                Ok(registry) => registry,
                // A boot that cannot prepare the login records still has to be
                // able to log in: the sessions move to memory, and only the
                // file view is lost.
                Err(err) => {
                    eprintln!(
                        "logind: cannot start the login records in {path}: {err}; \
                         keeping the sessions in memory"
                    );
                    Registry::new(UtmpFile::new(&path), boot_id.clone(), now)
                }
            };
            println!("logind: boot {boot_id} at {now}");

            let mut scheme = LogindScheme::new(
                "logind".to_owned(),
                LogindSchemeData::new(Rc::new(RefCell::new(registry))),
                LogindResource::Root(Root),
            );
            let handler = Blocking::new(&*socket, 16);

            let _ = daemon.ready_sync_scheme(&socket, &mut scheme);

            handler
                .process_requests_blocking(scheme)
                .expect("logind: failed to serve the login manager");
        })
    }
}

#[cfg(target_os = "redox")]
fn main() {
    daemon::main();
}

/// Nothing to manage when there are no sessions to keep: the daemon is part of
/// the Redox boot, and the crate still has to build for the host so that its
/// logic can be tested there.
#[cfg(not(target_os = "redox"))]
fn main() {
    eprintln!("logind: the login manager only runs on Redox");
    std::process::exit(1);
}
