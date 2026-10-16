//! The `/scheme/logind` interface of the login manager.
//!
//! The scheme is how the rest of the system talks to the daemon. Reading a file
//! answers with the live state, which is what `loginctl` shows; writing to a
//! control file is a request, and the answer is read back from the descriptor
//! the request was written to.
//!
//! | file        | read                             | write                |
//! |-------------|----------------------------------|----------------------|
//! | `sessions`  | the live sessions, as records    | —                    |
//! | `seats`     | the seats, one line each         | —                    |
//! | `login`     | —                                | a login is starting  |
//! | `logout`    | —                                | a login is over      |
//! | `activate`  | —                                | take the console     |
//! | `terminate` | —                                | stop a session       |
//!
//! A read is a snapshot taken when the file is opened. The daemon handles one
//! request at a time, so the state cannot change under a reader halfway through
//! a listing.
//!
//! Requests are applied by [`crate::registry`], and what they mean for the
//! system — signalling a process — is done here, where the daemon can do it.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use redox_scheme::CallerCtx;
use scheme_utils::{resource_scheme, FpathWriter, ResourceOpenResult, ResourceSync};
use syscall::dirent::{DirEntry, DirentBuf, DirentKind};
use syscall::error::*;
use syscall::schemev2::NewFdFlags;
use syscall::Stat;

use crate::protocol::{encode_seats, format_response, Request};
use crate::registry::Registry;

/// The files of the scheme, in the order `ls` lists them.
pub const ENTRIES: [&str; 6] = [
    "sessions",
    "seats",
    "login",
    "logout",
    "activate",
    "terminate",
];

resource_scheme! {
    pub LogindScheme<>;
    type SchemeData = LogindSchemeData;

    pub enum LogindResource {
        Root(Root),
        Control(Control),
        Text(Text),
    }
}

/// The state every resource of the scheme shares: the sessions of this boot.
pub struct LogindSchemeData {
    registry: Rc<RefCell<Registry>>,
}

impl LogindSchemeData {
    /// A scheme over a running registry.
    pub fn new(registry: Rc<RefCell<Registry>>) -> Self {
        Self { registry }
    }
}

/// The root of the scheme: it opens the files, and lists them.
#[derive(Debug)]
pub struct Root;

impl ResourceSync for Root {
    type ResourceEnum = LogindResource;
    type SchemeData = LogindSchemeData;

    fn openat(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        path: &str,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<ResourceOpenResult<Self::ResourceEnum>> {
        let resource = match path {
            "sessions" => {
                let registry = scheme_data.registry.borrow();
                LogindResource::Text(Text::new("sessions", registry.sessions().encode()))
            }
            "seats" => {
                let registry = scheme_data.registry.borrow();
                LogindResource::Text(Text::new(
                    "seats",
                    encode_seats(&registry.sessions().seats()),
                ))
            }
            // The control files: each one takes the request that names it.
            "login" => LogindResource::Control(Control::new("login")),
            "logout" => LogindResource::Control(Control::new("logout")),
            "activate" => LogindResource::Control(Control::new("activate")),
            "terminate" => LogindResource::Control(Control::new("terminate")),
            // The root itself is a directory, and so is anything that is not
            // one of the files above.
            "" => return Err(Error::new(EISDIR)),
            _ => return Err(Error::new(ENOENT)),
        };

        Ok(ResourceOpenResult::ThisScheme {
            data: resource,
            flags: NewFdFlags::empty(),
        })
    }

    fn getdents<'buf>(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        mut buf: DirentBuf<&'buf mut [u8]>,
        opaque_offset: u64,
    ) -> Result<DirentBuf<&'buf mut [u8]>> {
        for (index, name) in ENTRIES.iter().enumerate().skip(opaque_offset as usize) {
            buf.entry(DirEntry {
                inode: 0,
                next_opaque_id: index as u64 + 1,
                name,
                kind: DirentKind::Regular,
            })?;
        }

        Ok(buf)
    }

    fn fpath(&mut self, _scheme_data: &mut Self::SchemeData, w: &mut FpathWriter) -> Result<()> {
        // The root is the scheme itself: `/scheme/logind`.
        let _ = w;
        Ok(())
    }
}

/// A file that hands out a snapshot of the state.
#[derive(Debug)]
pub struct Text {
    name: &'static str,
    body: Vec<u8>,
    cursor: usize,
}

impl Text {
    fn new(name: &'static str, body: String) -> Self {
        Self {
            name,
            body: body.into_bytes(),
            cursor: 0,
        }
    }
}

impl ResourceSync for Text {
    type ResourceEnum = LogindResource;
    type SchemeData = LogindSchemeData;

    fn read(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &mut [u8],
        _offset: u64,
        _fcntl_flags: u32,
    ) -> Result<usize> {
        // The snapshot has its own position: a reader that seeks back to the
        // start asks for the file again, and gets it.
        let rest = &self.body[self.cursor.min(self.body.len())..];
        let count = rest.len().min(buf.len());
        buf[..count].copy_from_slice(&rest[..count]);
        self.cursor += count;

        Ok(count)
    }

    fn fsize(&mut self, _scheme_data: &mut Self::SchemeData) -> Result<u64> {
        Ok(self.body.len() as u64)
    }

    fn fpath(&mut self, _scheme_data: &mut Self::SchemeData, w: &mut FpathWriter) -> Result<()> {
        w.push_str(self.name);
        Ok(())
    }

    fn fstat(&mut self, _scheme_data: &mut Self::SchemeData, stat: &mut Stat) -> Result<()> {
        stat.st_size = self.body.len() as u64;
        Ok(())
    }
}

/// A file that takes a request and answers it.
#[derive(Debug)]
pub struct Control {
    name: &'static str,
    response: Vec<u8>,
    cursor: usize,
}

impl Control {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            response: Vec::new(),
            cursor: 0,
        }
    }
}

impl ResourceSync for Control {
    type ResourceEnum = LogindResource;
    type SchemeData = LogindSchemeData;

    fn read(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &mut [u8],
        _offset: u64,
        _fcntl_flags: u32,
    ) -> Result<usize> {
        let rest = &self.response[self.cursor.min(self.response.len())..];
        let count = rest.len().min(buf.len());
        buf[..count].copy_from_slice(&rest[..count]);
        self.cursor += count;

        Ok(count)
    }

    fn write(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        buf: &[u8],
        _offset: u64,
        _fcntl_flags: u32,
    ) -> Result<usize> {
        let request = String::from_utf8_lossy(buf);
        self.response = handle(scheme_data, &request).into_bytes();
        self.cursor = 0;

        Ok(buf.len())
    }

    fn fsize(&mut self, _scheme_data: &mut Self::SchemeData) -> Result<u64> {
        Ok(self.response.len() as u64)
    }

    fn fpath(&mut self, _scheme_data: &mut Self::SchemeData, w: &mut FpathWriter) -> Result<()> {
        w.push_str(self.name);
        Ok(())
    }
}

/// Applies one request and renders what the client reads back.
fn handle(scheme_data: &mut LogindSchemeData, text: &str) -> String {
    let request = match Request::parse(text) {
        Ok(request) => request,
        Err(err) => return format_response(&Err(err)),
    };

    let mut registry = scheme_data.registry.borrow_mut();

    // A session whose leader is gone is not worth listing or signalling, and a
    // request is the moment to notice: the session that died has to be out of
    // the way before the next one takes its terminal.
    registry.prune(|pid| !gone(pid));

    match registry.handle(&request, journald::time::unix_now()) {
        Ok(outcome) => {
            if let Some(pid) = outcome.signal {
                signal(pid);
            }
            format_response(&Ok(outcome.message))
        }
        Err(err) => format_response(&Err(err)),
    }
}

/// Whether the kernel says the process is not there any more.
///
/// Only a refusal with `ESRCH` counts. Anything else — including a signal the
/// kernel does not implement — is not proof that a session is over, and a live
/// session stays listed: showing a finished session is a smaller problem than
/// hiding a live one.
fn gone(pid: i32) -> bool {
    if pid <= 0 {
        return true;
    }

    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return false;
    }

    matches!(
        io::Error::last_os_error().raw_os_error(),
        Some(code) if code == libc::ESRCH
    )
}

/// Asks a session leader to stop.
///
/// The scheme cannot do more than this: without process groups, stopping a
/// session means stopping its leader, and what it started is up to it.
fn signal(pid: i32) {
    if pid <= 0 {
        return;
    }

    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
        eprintln!("logind: cannot stop {pid}: {}", io::Error::last_os_error());
    }
}
