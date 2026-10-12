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
use crate::pairing::Pairing;

pub const ID_ROOT: usize = 1;
const ID_CTL: usize = 2;
const ID_STATUS: usize = 3;
const ID_EVENT: usize = 4;
const ID_DEVICES: usize = 5;
const ID_ACL: usize = 6;

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
            pairing: Pairing::new(),
            l2cap: crate::l2cap::ChannelTable::new(),
            acl_in: Vec::new(),
            acl_out: Vec::new(),
            pairing_handle: None,
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
    /// estado do pareamento (SSP/PIN) atual
    pub pairing: Pairing,
    /// canais L2CAP dinamicos
    pub l2cap: crate::l2cap::ChannelTable,
    /// fila de pacotes ACL recebidos p/ userspace ([dcid u16 LE][payload])
    pub acl_in: Vec<Vec<u8>>,
    /// fila de pacotes ACL p/ envio (daemon drena)
    pub acl_out: Vec<(u16, Vec<u8>)>, // (dcid/sig-cid, payload)
    /// connection handle do link pareado (necessario p/ abrir canais)
    pub pairing_handle: Option<u16>,
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
        // Eventos de pareamento: roteia p/ pairing e coleta comandos de volta
        let emitted = self.pairing.on_event(code, params);
        self.pending_cmds.extend(emitted);
        if self.pairing_handle.is_none() {
            self.pairing_handle = self.pairing.connection_handle();
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
            _ if cmd.starts_with("pair ") => {
                let bd = Pairing::parse_bd_text(cmd[5..].trim())
                    .ok_or(Error::new(EINVAL))?;
                if !self.ready {
                    return Err(Error::new(EAGAIN));
                }
                self.pending_cmds.push(self.pairing.start_pair(bd));
                return Ok(());
            }
            _ if cmd.starts_with("open ") => {
                let psm: u16 = cmd[5..]
                    .trim()
                    .parse()
                    .map_err(|_| Error::new(EINVAL))?;
                let bd = self.pairing.paired_device().ok_or(Error::new(EAGAIN))?;
                let _ = bd;
                let handle = self
                    .pairing_handle
                    .ok_or_else(|| Error::new(EINVAL))?;
                let (ch, sig) = self.l2cap.open(handle, psm, 1);
                log::info!("btusd: abrindo canal L2CAP psm={} scid={:#06x}", psm, ch.scid);
                self.acl_out.push((
                    crate::l2cap::CID_SIGNALING,
                    sig.encode(),
                ));
                return Ok(());
            }
            _ if cmd.starts_with("pin ") => {
                // configura PIN legado SEM emitir HCI agora (usado se o par pedir)
                let pin = cmd[4..].trim();
                if pin.is_empty() || pin.len() > 16 || !pin.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::new(EINVAL));
                }
                self.pairing.set_pin(pin.to_string());
                return Ok(());
            }
            _ => return Err(Error::new(EINVAL)),
        };
        self.pending_cmds.push(hci);
        Ok(())
    }

    /// Pacote ACL recebido do controlador. Sinalizacao vai p/ L2CAP;
    /// dados de canais abertos sobem p/ userspace como [dcid LE][payload].
    pub fn handle_acl(&mut self, pkt: &[u8]) {
        let Some((acl, _rest)) = crate::l2cap::decode_acl(pkt) else {
            return;
        };
        if !self.ready {
            return;
        }
        // roteia pelo payload L2CAP
        if acl.payload.len() < 4 {
            return;
        }
        let dcid = u16::from_le_bytes([acl.payload[2], acl.payload[3]]);
        if dcid == crate::l2cap::CID_SIGNALING {
            self.handle_sig(&acl);
            return;
        }
        // canal aberto? valida e enfileira cru p/ userspace
        if self.l2cap.route(&acl).is_ok() {
            self.acl_in.push(pkt.to_vec());
        }
    }

    /// L2CAP signaling sobre CID 1.
    fn handle_sig(&mut self, acl: &crate::l2cap::AclPacket) {
        let Some(sig) = crate::l2cap::SigPacket::decode(&acl.payload[4..]) else {
            return;
        };
        match sig.code {
            crate::l2cap::SIG_CONN_RSP => {
                if sig.data.len() < 8 { return; }
                let dcid = u16::from_le_bytes([sig.data[0], sig.data[1]]);
                let scid = u16::from_le_bytes([sig.data[2], sig.data[3]]);
                let result = u16::from_le_bytes([sig.data[4], sig.data[5]]);
                if let Ok((dcid, config_req)) = self.l2cap.on_conn_rsp(scid, dcid, result, sig.ident) {
                    self.acl_out.push((crate::l2cap::CID_SIGNALING, config_req.encode()));
                    let _ = dcid;
                }
            }
            crate::l2cap::SIG_CONFIG_RSP => {
                if sig.data.len() < 6 { return; }
                let dcid = u16::from_le_bytes([sig.data[2], sig.data[3]]);
                let result = u16::from_le_bytes([sig.data[4], sig.data[5]]);
                let _ = self.l2cap.on_config_rsp(dcid, result);
            }
            _ => {}
        }
    }

    /// Comandos pendentes p/ o daemon despachar via ACL bulk (dcid sinal = CID 1).
    pub fn drain_acl_out(&mut self) -> impl Iterator<Item = (u16, Vec<u8>)> + '_ {
        self.acl_out.drain(..)
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
            "state={} hci={} bd={} inquiring={} devices={} pair={:?}\n",
            if self.ready { "ready" } else { "probe" },
            self.hci_version,
            bd.as_deref().unwrap_or("--"),
            self.inquiring as u8,
            self.devices.len(),
            self.pairing.phase(),
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
            "acl" => (ID_ACL, NewFdFlags::empty()),
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
            ID_ACL => {
                if self.shared.acl_in.is_empty() {
                    let nonblock = flags & syscall::O_NONBLOCK as u32 != 0;
                    return Err(Error::new(if nonblock { EAGAIN } else { EWOULDBLOCK }));
                }
                let ev = self.shared.acl_in.remove(0);
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
        if id == ID_ACL {
            // dado cru de userspace: [dcid u16 LE][payload]
            if buf.len() < 2 {
                return Err(Error::new(EINVAL));
            }
            let dcid = u16::from_le_bytes([buf[0], buf[1]]);
            self.shared.acl_out.push((dcid, buf[2..].to_vec()));
            return Ok(buf.len());
        }
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

    /// E2E no nivel scheme: "pair xx:.." via ctl -> eventos JustWorks ->
    /// respostas em pending_cmds (como o daemon drenaria) -> Paired.
    #[test]
    fn pairing_e2e_via_ctl_e_eventos() {
        use crate::pairing::PairPhase;
        let mut s = ready_shared();

        s.ctl_command("pair de:ad:be:ef:00:42").unwrap();
        let cmd = s.drain_commands().next().unwrap();
        assert_eq!(cmd.opcode, 0x0405); // Create Connection
        assert_eq!(&cmd.params[..6], &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42]);

        let bd = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42];

        let mut ev = |code: u8, params: &[u8]| {
            let mut pkt = vec![code, params.len() as u8];
            pkt.extend_from_slice(params);
            s.handle_hci_event(&pkt);
            s.drain_commands().map(|c| c.opcode).collect::<Vec<u16>>()
        };

        // 1) Connection Complete (handle 0x20)  -> AuthRequested
        let mut p = vec![0x00];
        p.extend_from_slice(&0x20u16.to_le_bytes());
        p.extend_from_slice(&bd);
        p.extend_from_slice(&[1, 0]);
        let ops = ev(0x03, &p);
        assert_eq!(ops, vec![0x0411]);

        // 2) Link Key Req (sem chave) -> NegReply; 3) IO Cap -> Reply;
        //    4) User Confirm -> Reply
        assert_eq!(ev(0x17, &bd), vec![0x040C]);
        assert_eq!(ev(0x31, &bd), vec![0x042B]);
        let mut uc = bd.to_vec();
        uc.extend_from_slice(&111222u32.to_le_bytes());
        assert_eq!(ev(0x33, &uc), vec![0x042C]);

        // 5) SSP Complete ok -> SetConnEncrypt(0x20)
        let mut ssp = vec![0x00];
        ssp.extend_from_slice(&bd);
        assert_eq!(ev(0x36, &ssp), vec![0x0413]);

        // 6) Link Key notif -> sem comando, mas guarda chave
        let mut kn = bd.to_vec();
        kn.extend_from_slice(&[0x5A; 16]);
        kn.push(4);
        assert!(ev(0x18, &kn).is_empty());

        // 7) Encryption Change ok -> Paired
        assert!(ev(0x08, &[0x00, 0x20, 0x00]).is_empty());

        // pin e re-uso: proxima conexao usa a link key guardada
        let ops = ev(0x17, &bd);
        assert_eq!(ops, vec![0x040B]); // LinkKey REPLY com a chave
        drop(ev); // encerra o borrow mutavel
        assert_eq!(s.pairing.phase(), PairPhase::Paired);

        assert!(s.status_line().contains("pair=Paired"));
    }

    fn fed_paired(s: &mut BtShared) {
        s.ctl_command("pair de:ad:be:ef:00:42").unwrap();
        s.drain_commands().for_each(drop);
        let bd = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42];
        let mut feed = |code: u8, params: &[u8]| {
            let mut pkt = vec![code, params.len() as u8];
            pkt.extend_from_slice(params);
            s.handle_hci_event(&pkt);
            s.drain_commands().for_each(drop);
        };
        let mut p = vec![0x00];
        p.extend_from_slice(&0x20u16.to_le_bytes());
        p.extend_from_slice(&bd);
        p.extend_from_slice(&[1, 0]);
        feed(0x03, &p); // conn complete
        feed(0x17, &bd);
        feed(0x31, &bd);
        let mut uc = bd.to_vec();
        uc.extend_from_slice(&0u32.to_le_bytes());
        feed(0x33, &uc);
        let mut ssp = vec![0x00];
        ssp.extend_from_slice(&bd);
        feed(0x36, &ssp);
        let mut kn = bd.to_vec();
        kn.extend_from_slice(&[0x5A; 16]);
        kn.push(4);
        feed(0x18, &kn);
        feed(0x08, &[0x00, 0x20, 0x00]);
        assert_eq!(s.pairing.paired_device(), Some(bd));
        assert_eq!(s.pairing_handle, Some(0x20));
    }

    #[test]
    fn open_psm_sem_parear_falha() {
        let mut s = ready_shared();
        assert!(s.ctl_command("open 1").is_err());
    }

    #[test]
    fn canal_l2cap_sdp_abre_e_troca_dados() {
        let mut s = ready_shared();
        fed_paired(&mut s);

        // open psm=1 (SDP) -> acl_out recebe o ConnReq em CID 1
        s.ctl_command("open 1").unwrap();
        let (cid, sigpkg) = s.drain_acl_out().next().unwrap();
        assert_eq!(cid, crate::l2cap::CID_SIGNALING);
        let sig = crate::l2cap::SigPacket::decode(&sigpkg).unwrap();
        assert_eq!(sig.code, crate::l2cap::SIG_CONN_REQ);
        assert_eq!(u16::from_le_bytes([sig.data[0], sig.data[1]]), 1); // SDP
        let scid = u16::from_le_bytes([sig.data[2], sig.data[3]]);
        assert_eq!(scid, 0x0040);

        // remoto aceita: ConnRsp [dcid_remoto=0x50][scid=0x40][result 0][status 0]
        let mut sigdata = Vec::new();
        sigdata.extend_from_slice(&0x0050u16.to_le_bytes());
        sigdata.extend_from_slice(&scid.to_le_bytes());
        sigdata.extend_from_slice(&0u16.to_le_bytes());
        sigdata.extend_from_slice(&0u16.to_le_bytes());
        let rsp = crate::l2cap::SigPacket { code: crate::l2cap::SIG_CONN_RSP, ident: 1, data: sigdata }.encode();

        let mut aclp = Vec::new();
        aclp.extend_from_slice(&(rsp.len() as u16).to_le_bytes());
        aclp.extend_from_slice(&crate::l2cap::CID_SIGNALING.to_le_bytes());
        aclp.extend_from_slice(&rsp);
        let acl_pkt = crate::l2cap::AclPacket::new(0x20, 2, 0, aclp).encode();
        s.handle_acl(&acl_pkt);
        // ConfigReq sai automaticamente
        let (cid2, cfgpkg) = s.drain_acl_out().next().unwrap();
        assert_eq!(cid2, crate::l2cap::CID_SIGNALING);
        assert_eq!(crate::l2cap::SigPacket::decode(&cfgpkg).unwrap().code, crate::l2cap::SIG_CONFIG_REQ);

        // ConfigRsp confirma: [flags][dcid=0x50][result=0]
        let mut cfgr = Vec::new();
        cfgr.extend_from_slice(&0u16.to_le_bytes());
        cfgr.extend_from_slice(&0x0050u16.to_le_bytes());
        cfgr.extend_from_slice(&0u16.to_le_bytes());
        let cfgrsp = crate::l2cap::SigPacket { code: crate::l2cap::SIG_CONFIG_RSP, ident: 2, data: cfgr.clone() }.encode();
        let mut aclp2 = Vec::new();
        aclp2.extend_from_slice(&(cfgrsp.len() as u16).to_le_bytes());
        aclp2.extend_from_slice(&crate::l2cap::CID_SIGNALING.to_le_bytes());
        aclp2.extend_from_slice(&cfgrsp);
        let pkt2 = crate::l2cap::AclPacket::new(0x20, 2, 0, aclp2).encode();
        s.handle_acl(&pkt2);

        // canal aberto: dados de userspace saem embrulhados em ACL
        let out_acl = s.l2cap.encode_for(0x0050, b"hello remote").unwrap();
        assert_eq!(out_acl.handle, 0x20);
        assert_eq!(out_acl.pb, 2);
        assert_eq!(out_acl.payload[4], b'h');

        // dados internos do remoto disparam acl_in (userspace le "acl")
        let mut aclp3 = Vec::new();
        aclp3.extend_from_slice(&5u16.to_le_bytes());
        aclp3.extend_from_slice(&0x0050u16.to_le_bytes());
        aclp3.extend_from_slice(b"WORLD");
        let pkt3 = crate::l2cap::AclPacket::new(0x20, 2, 0, aclp3).encode();
        s.handle_acl(&pkt3);
        assert_eq!(s.acl_in.len(), 1);
        let dec = crate::l2cap::decode_acl(&s.acl_in[0]).unwrap().0;
        assert_eq!(&dec.payload[4..], b"WORLD");
        // canal nao aberto e rejeitado
        let mut bad = Vec::new();
        bad.extend_from_slice(&1u16.to_le_bytes());
        bad.extend_from_slice(&0x0099u16.to_le_bytes());
        bad.push(0x42);
        let pkt4 = crate::l2cap::AclPacket::new(0x20, 2, 0, bad).encode();
        s.handle_acl(&pkt4);
        assert_eq!(s.acl_in.len(), 1);
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
