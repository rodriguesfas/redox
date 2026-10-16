//! The `/scheme/networkd` interface of the network daemon.
//!
//! The scheme is how the rest of the boot talks to the daemon. Reading `status`
//! answers with the live report, which is what the daemon sees right now;
//! writing to a control file is a request, and the answer is read back from the
//! descriptor the request was written to.
//!
//! | file     | read            | write                        |
//! |----------|-----------------|------------------------------|
//! | `status` | the live report | —                            |
//! | `apply`  | —               | make the stack match it      |
//!
//! A read is a snapshot taken when the file is opened. Requests are applied by
//! [`crate::service::Service`], which is where the work is tested; the scheme
//! only turns a request into an answer.

use redox_scheme::CallerCtx;
use scheme_utils::{resource_scheme, FpathWriter, ResourceOpenResult, ResourceSync};
use syscall::dirent::{DirEntry, DirentBuf, DirentKind};
use syscall::error::*;
use syscall::schemev2::NewFdFlags;
use syscall::Stat;

use crate::protocol::{format_response, Request};
use crate::service::Service;

/// The files of the scheme, in the order `ls` lists them.
pub const ENTRIES: [&str; 2] = ["status", "apply"];

resource_scheme! {
    pub NetworkdScheme<>;
    type SchemeData = NetworkdSchemeData;

    pub enum NetworkdResource {
        Root(Root),
        Control(Control),
        Text(Text),
    }
}

/// The state every resource of the scheme shares: the configuration.
pub struct NetworkdSchemeData {
    service: Service,
}

impl NetworkdSchemeData {
    /// A scheme over a network configuration.
    pub fn new(service: Service) -> Self {
        Self { service }
    }
}

/// The root of the scheme: it opens the files, and lists them.
#[derive(Debug)]
pub struct Root;

impl ResourceSync for Root {
    type ResourceEnum = NetworkdResource;
    type SchemeData = NetworkdSchemeData;

    fn openat(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        path: &str,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<ResourceOpenResult<Self::ResourceEnum>> {
        let resource = match path {
            // The report is taken when it is asked for, so what a reader sees
            // is the state at the moment of the read, not at boot.
            "status" => {
                NetworkdResource::Text(Text::new("status", scheme_data.service.status_document()))
            }
            // The control files: each one takes the request that names it.
            "apply" => NetworkdResource::Control(Control::new("apply")),
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
        // The root is the scheme itself: `/scheme/networkd`.
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
    type ResourceEnum = NetworkdResource;
    type SchemeData = NetworkdSchemeData;

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
    type ResourceEnum = NetworkdResource;
    type SchemeData = NetworkdSchemeData;

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
        self.response = handle(&scheme_data.service, &request).into_bytes();
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
fn handle(service: &Service, text: &str) -> String {
    match Request::parse(text) {
        Ok(request) => format_response(&service.handle(&request)),
        Err(err) => format_response(&Err(err)),
    }
}
