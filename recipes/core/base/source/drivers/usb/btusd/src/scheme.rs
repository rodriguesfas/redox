//! Scheme `bt` — controle userspace do daemon btusd (probe jogado p/ userspace).
//!
//! Arquivos (ex.: `/scheme/bt/`):
//! - `ctl`     — write: "inquiry" | "inquiry_cancel" | "bdaddr" | "reset"
//! - `status`  — read: linha "state=ready hci=5.3 bd=04:ca:38:02:aa:bb inquiring=1\n"
//! - `devices` — read: linhas "04ca3802aabb rssi=-40\n" do ultimo inquiry
//! - `event`   — read: proximo evento HCI cru (EWOULDBLOCK se vazio)
//!
//! O daemon drena comandos via `drain_commands()` e os envia ao controlador.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use redox_scheme::{CallerCtx, OpenResult, scheme::SchemeSync};
use syscall::error::{EAGAIN, EACCES, EINVAL, Error, EWOULDBLOCK, Result};
use syscall::schemev2::NewFdFlags;

use crate::hci::{
    cmd_inquiry, CMD_INQUIRY_CANCEL, CMD_READ_BD_ADDR, CMD_RESET, EVT_INQUIRY_COMPLETE,
    EVT_INQUIRY_RESULT, EVT_INQUIRY_RESULT_RSSI, HciCommand, InquiryResult, parse_event,
    parse_inquiry_result, LAP_GIAC,
};

pub const ID_ROOT: usize = 1;
const ID_CTL: usize = 2;
const ID_STATUS: usize = 3;
const ID_EVENT: usize = 4;
const ID_DEVICES: usize = 5;

/// Estado compartilhado do scheme com o loop do daemon.
#[derive(Debug, Default)]
pub struct BtSharedCoreOptionless;

impl Default for BtShared {
    fn default() -> Self {
        Self {
            ready: false,
            hci_version: "?",
            bd_addr: None,
            manufacturer: 0,
            inquiring: false,
            devices: Vec::new(),
            events: Vec::new(),
            pending_cmds: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct BtShared {
    /// pronto depois do probe
    pub ready: bool,
    pub hci_version: &'static str,
    pub bd_addr: Option<[u8; 6]>,
    pub manufacturer: u16,
    pub inquiring: bool,
    pub devices: Vec<InquiryResult>,
    pub events: Vec<Vec<u8>>,
    pub pending_cmds: Vec<HciCommand>,
}

impl BtShared {
    /// Feed de um pacote de evento HCI vindo da USB (chamado pelo loop).
    pub fn handle_hci_event(&mut self, pkt: &[u8]) {
        let Some((code, params)) = parse_event(pkt) else {
            return;
        };
        match code {
            EVT_INQUIRY_RESULT | EVT_INQUIRY_RESULT_RSSI => {
                if let Some(found) = parse_inquiry_result(code, params) {
                    for dev in found {
                        if !self.devices.iter().any(|d| d.bd_addr == dev.bd_addr) {
                            self.devices.push(dev);
                        }
                    }
                }
            }
            EVT_INQUIRY_COMPLETE => {
                self.inquiring = false;
            }
            _ => {}
        }
        // guarda cru p/ userspace decidir
        self.events.push(pkt.to_vec());
    }

    /// Processa comando do ctl. Retorna a acao a tomar (comada HCI a enviar).
    pub fn ctl_command(&mut self, cmd: &str) -> Result<()> {
        let cmd = cmd.trim();
        let hci = match cmd {
            "inquiry" => {
                if !self.ready {
                    return Err(Error::new(EAGAIN));
                }
                self.inquiring = true;
                // GIAC, duracao ~10s (8 x 1.28s), respostas ilimitadas
                cmd_inquiry(LAP_GIAC, 8, 0)
            }
            "inquiry_cancel" => {
                self.inquiring = false;
                HciCommand::new(CMD_INQUIRY_CANCEL)
            }
            "bdaddr" => HciCommand::new(CMD_READ_BD_ADDR),
            "reset" => {
                self.devices.clear();
                self.inquiring = false;
                HciCommand::new(CMD_RESET)
            }
            _ => return Err(Error::new(EINVAL)),
        };
        self.pending_cmds.push(hci);
        Ok(())
    }

    /// Comandos prontos p/ o daemon enviar ao controlador.
    pub fn drain_commands(&mut self) -> impl Iterator<Item = HciCommand> + '_ {
        self.pending_cmds.drain(..)
    }

    pub fn status_line(&self) -> String {
        let mut s = String::new();
        let bd = self.bd_addr.map(|a| {
            format_args!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                a[0], a[1], a[2], a[3], a[4], a[5]
            )
            .to_string()
        });
        let _ = write!(
            s,
            "state={} hci={} bd={} inquiring={} devices={}\n",
            if self.ready { "ready" } else { "probe" },
            self.hci_version,
            bd.as_deref().unwrap_or("--"),
            self.inquiring as u8,
            self.devices.len(),
        );
        s
    }

    pub fn devices_list(&self) -> String {
        let mut s = String::new();
        for d in &self.devices {
            let a = d.bd_addr;
            let _ = writeln!(
                s,
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} rssi={:?}",
                a[0], a[1], a[2], a[3], a[4], a[5],
                d.rssi_dbm,
            );
        }
        s
    }
}

/// Scheme `bt` (Socket-based, SchemeSync direto, estilo zerod).
pub struct BtScheme {
    pub shared: BtShared,
}

impl BtScheme {
    pub fn new(shared: BtShared) -> Self {
        Self { shared }
    }
}

impl SchemeSync for BtScheme {
    fn scheme_root(&mut self) -> Result<usize> {
        Ok(ID_ROOT)
    }

    fn openat(
        &mut self,
        dirfd: usize,
        path: &str,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<OpenResult> {
        if dirfd != ID_ROOT {
            return Err(Error::new(EACCES));
        }
        let (id, flags) = match path {
            "" | "status" => (ID_STATUS, NewFdFlags::POSITIONED),
            "ctl" => (ID_CTL, NewFdFlags::POSITIONED),
            "event" => (ID_EVENT, NewFdFlags::POSITIONED),
            "devices" => (ID_DEVICES, NewFdFlags::POSITIONED),
            _ => return Err(Error::new(EINVAL)),
        };
        Ok(OpenResult::ThisScheme { number: id, flags })
    }

    fn read(
        &mut self,
        id: usize,
        buf: &mut [u8],
        offset: u64,
        flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        match id {
            ID_STATUS => {
                let data = self.shared.status_line().into_bytes();
                let start = offset as usize;
                if start >= data.len() {
                    return Ok(0);
                }
                let n = core::cmp::min(buf.len(), data.len() - start);
                buf[..n].copy_from_slice(&data[start..start + n]);
                Ok(n)
            }
            ID_DEVICES => {
                let data = self.shared.devices_list().into_bytes();
                let start = offset as usize;
                if start >= data.len() {
                    return Ok(0);
                }
                let n = core::cmp::min(buf.len(), data.len() - start);
                buf[..n].copy_from_slice(&data[start..start + n]);
                Ok(n)
            }
            ID_EVENT => {
                if self.shared.events.is_empty() {
                    let nonblock = flags & syscall::O_NONBLOCK as u32 != 0;
                    return Err(Error::new(if nonblock { EAGAIN } else { EWOULDBLOCK }));
                }
                let ev = self.shared.events.remove(0);
                let n = core::cmp::min(ev.len(), buf.len());
                buf[..n].copy_from_slice(&ev[..n]);
                Ok(n)
            }
            ID_CTL => Err(Error::new(EINVAL)),
            _ => Err(Error::new(EINVAL)),
        }
    }

    fn write(
        &mut self,
        id: usize,
        buf: &[u8],
        _offset: u64,
        _flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        if id != ID_CTL {
            return Err(Error::new(EINVAL));
        }
        let cmd = core::str::from_utf8(buf).map_err(|_| Error::new(EINVAL))?;
        self.shared.ctl_command(cmd)?;
        Ok(buf.len())
    }

    fn fpath(&mut self, _id: usize, buf: &mut [u8], _ctx: &CallerCtx) -> Result<usize> {
        const NAME: &[u8] = b"bt";
        let n = core::cmp::min(NAME.len(), buf.len());
        buf[..n].copy_from_slice(&NAME[..n]);
        Ok(n)
    }

    fn fsync(&mut self, _id: usize, _ctx: &CallerCtx) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn ready_shared() -> BtShared {
        BtShared {
            ready: true,
            bd_addr: Some([0x04, 0xCA, 0x38, 0x02, 0xAA, 0xBB]),
            hci_version: "5.3",
            manufacturer: 0x0884,
            ..Default::default()
        }
    }

    #[test]
    fn ctl_inquiry_enfileira_e_marca() {
        let mut s = ready_shared();
        s.ctl_command("inquiry").unwrap();
        assert!(s.inquiring);
        let cmd = s.drain_commands().next().unwrap();
        assert_eq!(cmd.opcode, 0x0401); // INQUIRY
        // LAP GIAC LE
        assert_eq!(&cmd.params[..3], &[0x33, 0x8B, 0x9E]);
        assert!(s.drain_commands().next().is_none());
    }

    #[test]
    fn ctl_rejeita_desconhecido_e_scan_sem_probe() {
        let mut s = BtShared::default(); // nao ready
        assert!(s.ctl_command("inquiry").is_err());
        assert!(s.ctl_command("qualquer coisa").is_err());
        // reset sempre pode
        ready_shared().ctl_command("reset").unwrap();
    }

    #[test]
    fn eventos_inquiry_alimentam_devices_e_fila() {
        let mut s = ready_shared();
        // duas respostas num evento
        let mut params = vec![2u8];
        params.extend_from_slice(&[0x11; 6]);
        params.extend_from_slice(&[0x22; 6]);
        let mut pkt = vec![EVT_INQUIRY_RESULT, params.len() as u8];
        pkt.extend_from_slice(&params);
        s.handle_hci_event(&pkt);
        s.handle_hci_event(&pkt); // dup nao duplica
        assert_eq!(s.devices.len(), 2);
        assert_eq!(s.events.len(), 2); // crus na fila

        // complete desliga inquiring
        s.ctl_command("inquiry").unwrap();
        s.drain_commands().for_each(drop);
        s.handle_hci_event(&[EVT_INQUIRY_COMPLETE, 1, 0]);
        assert!(!s.inquiring);
    }

    #[test]
    fn status_line_format() {
        let s = ready_shared();
        let line = s.status_line();
        assert!(line.contains("state=ready"));
        assert!(line.contains("bd=04:ca:38:02:aa:bb"));
        assert!(line.contains("hci=5.3"));
    }
}
