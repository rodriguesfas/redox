//! The live state of the login manager.
//!
//! The registry is what the daemon keeps in memory: the sessions, and the
//! `utmp` record file kept in step with them. Every request a client sends is
//! applied here, so the state machine can be tested on the host — including the
//! fact that stopping a session means signalling a process, which is reported
//! back as an [`Outcome`] instead of being done in place.
//!
//! The file is written on every change rather than being regenerated: a
//! session appears in `utmp` as it starts, so a system that crashes mid-boot
//! still shows who was logged in.

use std::io;

use crate::protocol::{ProtocolError, Request};
use crate::session::{LoginRequest, Session, SessionType, Sessions};
use crate::utmp::{UtmpFile, UtmpRecord};

/// What the caller has to do on the system for a request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Message returned to the client, after `ok`.
    pub message: String,
    /// Pid to signal, for `terminate`.
    pub signal: Option<i32>,
}

/// The sessions of the current boot.
pub struct Registry {
    sessions: Sessions,
    utmp: UtmpFile,
    boot_id: String,
    boot_time: i64,
}

impl Registry {
    /// Starts the registry of a boot: the record file is truncated and the
    /// `BOOT_TIME` record written, so that a file left over from the previous
    /// boot cannot be mistaken for the current one.
    pub fn boot(utmp: UtmpFile, boot_id: impl Into<String>, now: i64) -> io::Result<Self> {
        utmp.reset()?;
        utmp.append(&UtmpRecord::boot_time(std::process::id() as i32, now))?;

        Ok(Self {
            sessions: Sessions::new(),
            utmp,
            boot_id: boot_id.into(),
            boot_time: now,
        })
    }

    /// Starts the registry without a record file to write to.
    ///
    /// This is the fallback for a boot where the login records cannot be
    /// prepared — a read-only root, say. Sessions are still managed, so logins
    /// and `loginctl` keep working; only the file view is missing, which is a
    /// far smaller problem than a login manager that refuses to start.
    pub fn new(utmp: UtmpFile, boot_id: impl Into<String>, now: i64) -> Self {
        Self {
            sessions: Sessions::new(),
            utmp,
            boot_id: boot_id.into(),
            boot_time: now,
        }
    }

    /// Identifier of this boot, shared with the journal.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }

    /// Seconds since the epoch when the daemon took over.
    pub fn boot_time(&self) -> i64 {
        self.boot_time
    }

    /// The sessions, in the order they started.
    pub fn sessions(&self) -> &Sessions {
        &self.sessions
    }

    /// The record file this registry keeps up to date.
    pub fn utmp(&self) -> &UtmpFile {
        &self.utmp
    }

    /// Applies a request.
    ///
    /// Errors are reported to the client and leave the registry alone, except
    /// for a failed write to the record file: the session is live in that case
    /// and only its record is missing, which is what the error says.
    pub fn handle(&mut self, request: &Request, now: i64) -> Result<Outcome, ProtocolError> {
        match request {
            Request::Login {
                user,
                uid,
                tty,
                leader,
                seat,
            } => self.login(user, *uid, tty, *leader, seat.clone(), now),
            Request::Logout { target } => self.logout(target),
            Request::Activate { target } => self.activate(target),
            Request::Terminate { target } => self.terminate(target),
        }
    }

    /// Drops the sessions whose leader is gone, returning their terminals.
    ///
    /// A session normally ends with a `logout` request from `login`; this is
    /// for the logins that never got that far, for instance because the process
    /// was killed. It is only ever called with a predicate that answers "gone"
    /// when it is sure: leaving a finished session listed is a smaller problem
    /// than hiding a live one.
    pub fn prune(&mut self, alive: impl Fn(i32) -> bool) -> Vec<String> {
        let dead: Vec<(String, String)> = self
            .sessions
            .list()
            .iter()
            .filter(|session| session.leader > 0 && !alive(session.leader))
            .map(|session| (session.id.clone(), session.tty.clone()))
            .collect();

        let mut removed = Vec::new();
        for (id, tty) in dead {
            self.sessions.logout(&id);
            let _ = self.utmp.remove(&tty);
            removed.push(tty);
        }
        removed
    }

    fn login(
        &mut self,
        user: &str,
        uid: u32,
        tty: &str,
        leader: i32,
        seat: Option<String>,
        now: i64,
    ) -> Result<Outcome, ProtocolError> {
        if tty.is_empty() {
            return Err(ProtocolError("a session needs a terminal".to_owned()));
        }

        // A terminal that logs in again takes its session over: a login that
        // was never reported as finished does not survive the next one.
        if let Some(previous) = self.sessions.session_for_tty(tty).map(|it| it.id.clone()) {
            self.sessions.logout(&previous);
        }

        let id = self.sessions.login(&LoginRequest {
            user: user.to_owned(),
            uid,
            tty: tty.to_owned(),
            leader,
            host: String::new(),
            session_type: SessionType::Tty,
            seat,
            now,
        });

        let session = self
            .sessions
            .session(&id)
            .expect("the session was just registered");
        self.utmp.put(&session.to_utmp()).map_err(|err| {
            ProtocolError(format!("session {id} is live but not recorded: {err}"))
        })?;

        Ok(Outcome {
            message: id,
            signal: None,
        })
    }

    fn logout(&mut self, target: &str) -> Result<Outcome, ProtocolError> {
        let (id, tty) = self.address(target)?;
        self.sessions.logout(&id);
        self.utmp
            .remove(&tty)
            .map_err(|err| ProtocolError(format!("cannot update the session records: {err}")))?;

        Ok(Outcome {
            message: tty,
            signal: None,
        })
    }

    fn activate(&mut self, target: &str) -> Result<Outcome, ProtocolError> {
        let (id, _) = self.address(target)?;
        if !self.sessions.activate(&id) {
            return Err(ProtocolError(format!("no such session: {id}")));
        }

        Ok(Outcome {
            message: id,
            signal: None,
        })
    }

    fn terminate(&mut self, target: &str) -> Result<Outcome, ProtocolError> {
        let (id, _) = self.address(target)?;
        let session = self
            .sessions
            .session(&id)
            .expect("the session was looked up by address");
        let leader = session.leader;
        self.sessions.set_closing(&id);

        Ok(Outcome {
            message: id,
            signal: (leader > 0).then_some(leader),
        })
    }

    /// Resolves a session identifier or a terminal name to a live session.
    fn address(&self, target: &str) -> Result<(String, String), ProtocolError> {
        let session = self.find(target)?;
        Ok((session.id.clone(), session.tty.clone()))
    }

    fn find(&self, target: &str) -> Result<&Session, ProtocolError> {
        self.sessions
            .session(target)
            .or_else(|| self.sessions.session_for_tty(target))
            .ok_or_else(|| ProtocolError(format!("no such session or terminal: {target}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> UtmpFile {
        let dir =
            std::env::temp_dir().join(format!("logind-registry-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        UtmpFile::new(dir.join("utmp"))
    }

    fn login(user: &str, uid: u32, tty: &str, leader: i32) -> Request {
        Request::Login {
            user: user.to_owned(),
            uid,
            tty: tty.to_owned(),
            leader,
            seat: None,
        }
    }

    #[test]
    fn booting_truncates_the_records() {
        let utmp = scratch("boot");
        utmp.put(&UtmpRecord::user_process(1, "tty0", "ghost", 1))
            .unwrap();

        let registry = Registry::boot(utmp, "boot-1", 1_000).unwrap();
        assert_eq!(registry.sessions().len(), 0);
        assert_eq!(registry.boot_id(), "boot-1");
        assert_eq!(registry.boot_time(), 1_000);

        // Only the boot record is left, and it names this boot.
        let records = registry.utmp().read_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record_type, crate::utmp::RecordType::BootTime);
        assert_eq!(records[0].time, 1_000);
    }

    #[test]
    fn a_fallback_boot_keeps_the_file_as_it_was() {
        let utmp = scratch("fallback");
        utmp.put(&UtmpRecord::user_process(1, "tty0", "ghost", 1))
            .unwrap();

        // Without a record file to prepare, the sessions are still managed.
        let mut registry = Registry::new(utmp, "boot-1b", 1_000);
        assert_eq!(registry.sessions().len(), 0);
        assert_eq!(registry.boot_id(), "boot-1b");
        assert_eq!(registry.boot_time(), 1_000);

        // Nothing was truncated: the boot record of the previous boot is not
        // there, and the record left behind is still the one it was.
        let records = registry.utmp().read_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].user, "ghost");

        // A login is still served, and still recorded.
        let outcome = registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        assert_eq!(outcome.message, "1");
        assert_eq!(registry.sessions().list()[0].user, "user");
    }

    #[test]
    fn a_login_lands_in_memory_and_in_the_file() {
        let mut registry = Registry::boot(scratch("login"), "boot-2", 1_000).unwrap();

        let outcome = registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        assert_eq!(outcome.message, "1");
        assert_eq!(outcome.signal, None);

        let session = &registry.sessions().list()[0];
        assert_eq!(session.user, "user");
        assert_eq!(session.uid, 1000);
        assert_eq!(session.tty, "tty1");
        assert_eq!(session.leader, 42);
        assert_eq!(session.seat, "seat0");
        assert!(session.is_active(), "the first login owns the console");

        // The file view agrees with the live view.
        let users = registry.utmp().read_users().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].line, "tty1");
        assert_eq!(users[0].user, "user");
        assert_eq!(users[0].pid, 42);
    }

    #[test]
    fn a_second_login_waits_for_the_console() {
        let mut registry = Registry::boot(scratch("second"), "boot-3", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        registry
            .handle(&login("root", 0, "tty2", 77), 1_200)
            .unwrap();

        assert_eq!(
            registry
                .sessions()
                .list()
                .iter()
                .map(|session| (session.id.as_str(), session.state.as_str()))
                .collect::<Vec<_>>(),
            vec![("1", "active"), ("2", "online")]
        );

        // Activating swaps which of the two owns the console.
        let outcome = registry
            .handle(
                &Request::Activate {
                    target: "2".to_owned(),
                },
                1_300,
            )
            .unwrap();
        assert_eq!(outcome.message, "2");
        assert_eq!(registry.sessions().active_on("seat0").unwrap().id, "2");
    }

    #[test]
    fn a_login_on_the_same_terminal_replaces_its_session() {
        let mut registry = Registry::boot(scratch("replace"), "boot-4", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        registry
            .handle(&login("root", 0, "tty1", 99), 1_200)
            .unwrap();

        assert_eq!(registry.sessions().len(), 1);
        assert_eq!(registry.sessions().list()[0].user, "root");
        assert_eq!(registry.utmp().read_users().unwrap().len(), 1);
        assert_eq!(registry.utmp().read_users().unwrap()[0].user, "root");
    }

    #[test]
    fn logging_out_clears_the_file_and_hands_the_console_over() {
        let mut registry = Registry::boot(scratch("logout"), "boot-5", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        registry
            .handle(&login("root", 0, "tty2", 77), 1_200)
            .unwrap();

        // Both by identifier and by terminal name.
        let outcome = registry
            .handle(
                &Request::Logout {
                    target: "1".to_owned(),
                },
                1_300,
            )
            .unwrap();
        assert_eq!(outcome.message, "tty1");
        assert_eq!(registry.utmp().read_users().unwrap().len(), 1);
        assert_eq!(
            registry.sessions().active_on("seat0").unwrap().id,
            "2",
            "the session that was waiting takes the console"
        );

        registry
            .handle(
                &Request::Logout {
                    target: "tty2".to_owned(),
                },
                1_400,
            )
            .unwrap();
        assert_eq!(registry.sessions().len(), 0);
        assert!(registry.utmp().read_users().unwrap().is_empty());
    }

    #[test]
    fn known_requests_are_answered_and_broken_ones_are_not() {
        let mut registry = Registry::boot(scratch("errors"), "boot-6", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();

        for request in [
            Request::Logout {
                target: "9".to_owned(),
            },
            Request::Activate {
                target: "tty9".to_owned(),
            },
            Request::Terminate {
                target: "9".to_owned(),
            },
        ] {
            let error = registry.handle(&request, 1_200).unwrap_err();
            assert!(error.0.contains("no such session"), "{error}");
        }

        let error = registry
            .handle(&login("user", 1000, "", 42), 1_200)
            .unwrap_err();
        assert!(error.0.contains("needs a terminal"), "{error}");
    }

    #[test]
    fn terminating_reports_the_process_to_signal() {
        let mut registry = Registry::boot(scratch("terminate"), "boot-7", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();

        let outcome = registry
            .handle(
                &Request::Terminate {
                    target: "tty1".to_owned(),
                },
                1_200,
            )
            .unwrap();
        assert_eq!(outcome.message, "1");
        assert_eq!(outcome.signal, Some(42));
        assert_eq!(registry.sessions().list()[0].state.as_str(), "closing");

        // A session with no leader is only marked as closing.
        let mut registry = Registry::boot(scratch("noleader"), "boot-8", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 0), 1_100)
            .unwrap();
        let outcome = registry
            .handle(
                &Request::Terminate {
                    target: "1".to_owned(),
                },
                1_200,
            )
            .unwrap();
        assert_eq!(outcome.signal, None);
    }

    #[test]
    fn finished_sessions_are_pruned_but_live_ones_are_kept() {
        let mut registry = Registry::boot(scratch("prune"), "boot-9", 1_000).unwrap();
        registry
            .handle(&login("user", 1000, "tty1", 42), 1_100)
            .unwrap();
        registry
            .handle(&login("root", 0, "tty2", 77), 1_200)
            .unwrap();

        let removed = registry.prune(|pid| pid == 42);
        assert_eq!(removed, vec!["tty2".to_owned()]);
        assert_eq!(registry.sessions().len(), 1);
        assert_eq!(registry.sessions().list()[0].tty, "tty1");
        assert_eq!(registry.utmp().read_users().unwrap().len(), 1);
    }
}
