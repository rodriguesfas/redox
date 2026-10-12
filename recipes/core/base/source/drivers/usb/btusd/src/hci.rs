//! Bluetooth HCI (Host Controller Interface) codec — HCI USB Transport (btusd).
//!
//! Pure logic: encodes HCI command packets and parses HCI events.
//! Formats per Bluetooth Core Spec v5.x, Vol 4 Part A (USB transport).

use alloc::vec::Vec;

/// HCI packet type byte for commands (USB transport does not use it, but the
/// buffer layout is shared with UART, so we keep it explicit).
pub const HCI_PKT_CMD: u8 = 0x01;

/// OGF/OCF packing per spec: opcode = (ogf << 10) | ocf.
pub const fn opcode(ogf: u8, ocf: u16) -> u16 {
    ((ogf as u16) << 10) | (ocf & 0x03FF)
}

// --- Commands we support in bring-up (OCF in OGF) ---
pub const OGF_LINK_CONTROL: u8 = 0x01;
pub const OGF_HC_BASEBAND: u8 = 0x03;
pub const OGF_INFORMATIONAL: u8 = 0x04;

/// HCI Reset
pub const CMD_RESET: u16 = opcode(OGF_HC_BASEBAND, 0x003); // 0x0C03
/// Read Local Version Information
pub const CMD_READ_LOCAL_VERSION: u16 = opcode(OGF_INFORMATIONAL, 0x001); // 0x1001
/// Read BD_ADDR
pub const CMD_READ_BD_ADDR: u16 = opcode(OGF_INFORMATIONAL, 0x009); // 0x1009
/// Set Event Mask (mask hints we enable: command complete/disconnect/inquiry etc.)
pub const CMD_SET_EVENT_MASK: u16 = opcode(OGF_HC_BASEBAND, 0x001); // 0x0C01

/// An HCI command packet ready to go on the control/bulk pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HciCommand {
    pub opcode: u16,
    pub params: Vec<u8>,
}

impl HciCommand {
    pub fn new(opcode: u16) -> Self {
        Self {
            opcode,
            params: Vec::new(),
        }
    }

    pub fn with_params(opcode: u16, params: Vec<u8>) -> Self {
        Self { opcode, params }
    }

    /// Serialize: [0x01][opcode LE16][param_len u8][params...]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.params.len());
        out.push(HCI_PKT_CMD);
        out.extend_from_slice(&self.opcode.to_le_bytes());
        out.push(self.params.len() as u8);
        out.extend_from_slice(&self.params);
        out
    }
}

/// HCI event code byte — Command Complete.
pub const EVT_COMMAND_COMPLETE: u8 = 0x0E;
/// HCI event — Command Status.
pub const EVT_COMMAND_STATUS: u8 = 0x0F;

/// Parsed HCI Command Complete event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandComplete {
    pub opcode: u16,
    /// Status byte (0 = success) plus true payload.
    pub status: u8,
    pub return_params: Vec<u8>,
}

/// Parse an HCI event packet payload (without the packet type byte).
/// Layout: [event_code u8][param_len u8][params...]
pub fn parse_event(pkt: &[u8]) -> Option<(u8, &[u8])> {
    if pkt.len() < 2 {
        return None;
    }
    let len = pkt[1] as usize;
    if pkt.len() < 2 + len {
        return None;
    }
    Some((pkt[0], &pkt[2..2 + len]))
}

/// Parse a Command Complete payload: [num_packets u8][opcode LE16][status u8][return...]
pub fn parse_command_complete(params: &[u8]) -> Option<CommandComplete> {
    if params.len() < 4 {
        return None;
    }
    let _num_hci_packets = params[0];
    let opcode = u16::from_le_bytes([params[1], params[2]]);
    let status = params[3];
    let return_params = params[4..].to_vec();
    Some(CommandComplete {
        opcode,
        status,
        return_params,
    })
}

/// Decoded Read Local Version answer (payload of Command Complete, minus status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalVersion {
    pub hci_version: u8,
    pub hci_subversion: u16,
    pub lmp_version: u8,
    pub manufacturer: u16,
    pub lmp_subversion: u16,
}

impl LocalVersion {
    pub fn parse(return_params: &[u8]) -> Option<Self> {
        if return_params.len() < 8 {
            return None;
        }
        Some(Self {
            hci_version: return_params[0],
            hci_subversion: u16::from_le_bytes([return_params[1], return_params[2]]),
            lmp_version: return_params[3],
            manufacturer: u16::from_le_bytes([return_params[4], return_params[5]]),
            lmp_subversion: u16::from_le_bytes([return_params[6], return_params[7]]),
        })
    }

    pub fn hci_version_str(&self) -> &'static str {
        match self.hci_version {
            0x09 => "5.0",
            0x0A => "5.1",
            0x0B => "5.2",
            0x0C => "5.3",
            0x0D => "5.4",
            _ => "?",
        }
    }
}

/// BD_ADDR is 6 bytes, little-endian on the wire.
pub fn parse_bd_addr(return_params: &[u8]) -> Option<[u8; 6]> {
    if return_params.len() < 6 {
        return None;
    }
    let mut addr = [0u8; 6];
    addr.copy_from_slice(&return_params[..6]);
    Some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn opcode_packing() {
        assert_eq!(CMD_RESET, 0x0C03);
        assert_eq!(CMD_READ_LOCAL_VERSION, 0x1001);
        assert_eq!(CMD_READ_BD_ADDR, 0x1009);
        assert_eq!(CMD_SET_EVENT_MASK, 0x0C01);
        // OGF/OCF fields independentes
        assert_eq!(opcode(0x3F, 0x3FF), 0xFFFF);
    }

    #[test]
    fn encode_reset() {
        let enc = HciCommand::new(CMD_RESET).encode();
        assert_eq!(enc, vec![0x01, 0x03, 0x0C, 0x00]); // type, op LE, len=0
    }

    #[test]
    fn encode_set_event_mask() {
        let mask = 0x0000_2000_0000_08FFu64.to_le_bytes().to_vec();
        let enc = HciCommand::with_params(CMD_SET_EVENT_MASK, mask).encode();
        assert_eq!(&enc[..3], &[0x01, 0x01, 0x0C]);
        assert_eq!(enc[3], 8); // param len
        assert_eq!(enc.len(), 4 + 8);
    }

    #[test]
    fn parse_command_complete_roundtrip() {
        // [code][len][num=1][opcode LE][status=0][payload...]
        let pkt = [0x0E, 5, 1, 0x03, 0x0C, 0x00, 0x42];
        let (code, params) = parse_event(&pkt).unwrap();
        assert_eq!(code, EVT_COMMAND_COMPLETE);
        let cc = parse_command_complete(params).unwrap();
        assert_eq!(cc.opcode, CMD_RESET);
        assert_eq!(cc.status, 0);
        assert_eq!(cc.return_params, vec![0x42]);
    }

    #[test]
    fn parse_rejeita_truncado() {
        assert!(parse_event(&[0x0E]).is_none());
        assert!(parse_event(&[0x0E, 9, 1]).is_none()); // len pede 9, tem 1
        assert!(parse_command_complete(&[1, 0x03]).is_none());
    }

    #[test]
    fn parse_local_version() {
        // resposta real de um controlador (status removido antes):
        let rp = [0x0C, 0x00, 0x22, 0x01, 0x84, 0x08, 0x09, 0x00];
        let lv = LocalVersion::parse(&rp).unwrap();
        assert_eq!(lv.hci_version, 0x0C); // 5.3
        assert_eq!(lv.hci_version_str(), "5.3");
        assert_eq!(lv.lmp_version, 0x01);
        assert_eq!(lv.manufacturer, 0x0884);

        assert!(LocalVersion::parse(&[0x0C]).is_none());
    }

    #[test]
    fn parse_bd_addr_le() {
        let rp = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        assert_eq!(parse_bd_addr(&rp).unwrap(), rp);
        assert!(parse_bd_addr(&rp[..5]).is_none());
    }
}

// ==== Inquiry (descoberta de dispositivos) ====

/// HCI Inquiry (OGF Link Control, OCF 0x0001) = 0x0401
pub const CMD_INQUIRY: u16 = opcode(OGF_LINK_CONTROL, 0x001);
/// HCI Inquiry Cancel = 0x0402
pub const CMD_INQUIRY_CANCEL: u16 = opcode(OGF_LINK_CONTROL, 0x002);

/// Local Unit LAP: GIAC (General Inquiry Access Code).
pub const LAP_GIAC: u32 = 0x9E8B33;

/// Monta o comando HCI Inquiry: [LAP LE24][duracao u8 em 1.28s][num_respostas].
pub fn cmd_inquiry(lap: u32, duration_128s: u8, max_responses: u8) -> HciCommand {
    let mut p = Vec::with_capacity(5);
    p.extend_from_slice(&lap.to_le_bytes()[..3]); // LAP sao 3 bytes LE
    p.push(duration_128s);
    p.push(max_responses);
    HciCommand::with_params(CMD_INQUIRY, p)
}

/// Eventos de inquiry.
pub const EVT_INQUIRY_COMPLETE: u8 = 0x01;
pub const EVT_INQUIRY_RESULT: u8 = 0x02; // resultado basico
pub const EVT_INQUIRY_RESULT_RSSI: u8 = 0x22; // com RSSI

/// Um dispositivo descoberto no inquiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InquiryResult {
    pub bd_addr: [u8; 6],
    /// -127..+20 dBm estimado (quando vem do evento com RSSI)
    pub rssi_dbm: Option<i8>,
}

/// Parse de Inquiry Result (com ou sem RSSI). Estado separado pelo codigo do evento.
/// Layout basic: [num u8][bd_addr[6] cada]...
/// Layout RSSI:  [num u8][bd_addr[6]][page_rep,page_period,page_mode][class 3][clock 2][rssi i8]... ainda
/// simplificamos: apenas extraimos os enderecos (especifico por-campo fica p/ depois).
pub fn parse_inquiry_result(event: u8, params: &[u8]) -> Option<Vec<InquiryResult>> {
    if params.is_empty() {
        return None;
    }
    let num = params[0] as usize;
    match event {
        EVT_INQUIRY_RESULT | EVT_INQUIRY_RESULT_RSSI => {
            let mut out = Vec::with_capacity(num);
            for i in 0..num {
                let off = 1 + i * 6;
                if off + 6 > params.len() {
                    return None;
                }
                let mut bd = [0u8; 6];
                bd.copy_from_slice(&params[off..off + 6]);
                out.push(InquiryResult {
                    bd_addr: bd,
                    rssi_dbm: None,
                });
            }
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod inquiry_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn inquiry_comando_layout() {
        let c = cmd_inquiry(LAP_GIAC, 0x30, 0);
        assert_eq!(c.opcode, 0x0401);
        // LAP LE 3B: 33 8B 9E
        assert_eq!(&c.params[..3], &[0x33, 0x8B, 0x9E]);
        assert_eq!(c.params[3], 0x30);
        let enc = c.encode();
        assert_eq!(&enc[..3], &[0x01, 0x01, 0x04]);
        assert_eq!(enc[3], 5); // plen
    }

    #[test]
    fn inquiry_result_parse() {
        // 2 dispositivos num evento basico
        let mut p = vec![2u8];
        p.extend_from_slice(&[0x11; 6]);
        p.extend_from_slice(&[0x22; 6]);
        let v = parse_inquiry_result(EVT_INQUIRY_RESULT, &p).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].bd_addr, [0x11; 6]);
        assert_eq!(v[1].bd_addr, [0x22; 6]);

        // truncado no meio do 2o endereco
        assert!(parse_inquiry_result(EVT_INQUIRY_RESULT, &p[..8]).is_none());
        // zero dispositivos = lista vazia (valido)
        assert_eq!(parse_inquiry_result(EVT_INQUIRY_RESULT, &[0]).unwrap().len(), 0);
        // outro evento
        assert!(parse_inquiry_result(0x0E, &p).is_none());
    }

    #[test]
    fn inquiry_opcodes() {
        assert_eq!(CMD_INQUIRY, 0x0401);
        assert_eq!(CMD_INQUIRY_CANCEL, 0x0402);
    }
}

// ==== Conexao / Pareamento (SSP + legado PIN) ====

// OGF 0x01 (Link Control) — opcodes reais por BT spec v5.x
/// Create Connection
pub const CMD_CREATE_CONNECTION: u16 = opcode(OGF_LINK_CONTROL, 0x005); // 0x0405
/// Accept Connection Request
pub const CMD_ACCEPT_CONNECTION: u16 = opcode(OGF_LINK_CONTROL, 0x009); // 0x0409
/// Reject Connection Request
pub const CMD_REJECT_CONNECTION: u16 = opcode(OGF_LINK_CONTROL, 0x00A); // 0x040A
/// Link Key Request Reply (com chave de 16B)
pub const CMD_LINK_KEY_REPLY: u16 = opcode(OGF_LINK_CONTROL, 0x00B); // 0x040B
/// Link Key Request Negative Reply
pub const CMD_LINK_KEY_NEGREPLY: u16 = opcode(OGF_LINK_CONTROL, 0x00C); // 0x040C
/// PIN Code Request Reply
pub const CMD_PIN_REPLY: u16 = opcode(OGF_LINK_CONTROL, 0x00D); // 0x040D
/// PIN Code Request Negative Reply
pub const CMD_PIN_NEGREPLY: u16 = opcode(OGF_LINK_CONTROL, 0x00E); // 0x040E
/// Authentication Requested
pub const CMD_AUTH_REQUESTED: u16 = opcode(OGF_LINK_CONTROL, 0x011); // 0x0411
/// Set Connection Encryption
pub const CMD_SET_CONN_ENCRYPT: u16 = opcode(OGF_LINK_CONTROL, 0x013); // 0x0413
/// Remote Name Request
pub const CMD_REMOTE_NAME_REQ: u16 = opcode(OGF_LINK_CONTROL, 0x019); // 0x0419

// OGF 0x03 (HC & Baseband)
/// Write Simple Pairing Mode (1 = enable SSP)
pub const CMD_WRITE_SSP_MODE: u16 = opcode(OGF_HC_BASEBAND, 0x056); // 0x0C56

// OGF 0x01 novamente (SSP inputs)
/// IO Capability Request Reply
pub const CMD_IO_CAP_REPLY: u16 = opcode(OGF_LINK_CONTROL, 0x02B); // 0x042B
/// User Confirmation Request Reply
pub const CMD_USER_CONFIRM_REPLY: u16 = opcode(OGF_LINK_CONTROL, 0x02C); // 0x042C
/// User Confirmation Negative Reply
pub const CMD_USER_CONFIRM_NEGREPLY: u16 = opcode(OGF_LINK_CONTROL, 0x02D); // 0x042D

// --- Eventos de conexao/SSP ---
pub const EVT_CONNECTION_COMPLETE: u8 = 0x03;
pub const EVT_CONNECTION_REQUEST: u8 = 0x04;
pub const EVT_AUTH_COMPLETE: u8 = 0x06;
pub const EVT_REMOTE_NAME_COMPLETE: u8 = 0x07;
pub const EVT_ENCRYPTION_CHANGE: u8 = 0x08;
pub const EVT_PIN_CODE_REQ: u8 = 0x16;
pub const EVT_LINK_KEY_REQ: u8 = 0x17;
pub const EVT_LINK_KEY_NOTIF: u8 = 0x18;
pub const EVT_IO_CAP_REQ: u8 = 0x31;
pub const EVT_USER_CONFIRM_REQ: u8 = 0x33;
pub const EVT_SSP_COMPLETE: u8 = 0x36;

/// Create Connection: [bd 6][packet_type u16 LE (0xCC18 = DH1/DH3/DH5/2DH*
/// mask comum)][page_scan_rep][page_scan_mode][clock_offset u16][allow_role]
pub fn cmd_create_connection(bd: [u8; 6]) -> HciCommand {
    let mut p = Vec::with_capacity(13);
    p.extend_from_slice(&bd);
    p.extend_from_slice(&0xCC18u16.to_le_bytes());
    p.push(0x01); // R1
    p.push(0x00); // mandatory page scan mode
    p.extend_from_slice(&0u16.to_le_bytes()); // clock offset desconhecido
    p.push(0x00); // nao aceita virar master? 0 = stay slave
    HciCommand::with_params(CMD_CREATE_CONNECTION, p)
}

/// Accept Connection Request: [bd][role]
pub fn cmd_accept_connection(bd: [u8; 6], role_slave: bool) -> HciCommand {
    HciCommand::with_params(
        CMD_ACCEPT_CONNECTION,
        {
            let mut v = Vec::with_capacity(7);
            v.extend_from_slice(&bd);
            v.push(u8::from(role_slave));
            v
        },
    )
}

/// Write SSP Mode param: 1 byte habilita
pub fn cmd_write_ssp_mode(enabled: bool) -> HciCommand {
    HciCommand::with_params(CMD_WRITE_SSP_MODE, alloc::vec![u8::from(enabled)])
}

/// PIN reply: [bd][pin_len][pin ASCII]
pub fn cmd_pin_reply(bd: [u8; 6], pin: &str) -> HciCommand {
    let mut v = Vec::new();
    v.extend_from_slice(&bd);
    v.push(pin.len() as u8);
    let bytes = pin.as_bytes();
    v.extend_from_slice(bytes.iter().copied().take(16).collect::<Vec<u8>>().as_slice());
    HciCommand::with_params(CMD_PIN_REPLY, v)
}

/// IO Cap Reply: [bd][io_cap][oob_present][auth_req]
/// JustWorks == io_cap=DisplayYesNo(0x01) ou NoInputNoOutput(0x03), oob=0, auth=0
pub fn cmd_io_cap_reply(bd: [u8; 6], io_cap: u8, auth_req: u8) -> HciCommand {
    let mut v = Vec::with_capacity(10);
    v.extend_from_slice(&bd);
    v.push(io_cap);
    v.push(0);
    v.push(auth_req);
    HciCommand::with_params(CMD_IO_CAP_REPLY, v)
}

pub const IO_CAP_DISPLAY_YESNO: u8 = 0x01;
pub const IO_CAP_NO_INPUT_NO_OUTPUT: u8 = 0x03;

/// User Confirm Reply: [bd]
pub fn cmd_user_confirm_reply(bd: [u8; 6]) -> HciCommand {
    HciCommand::with_params(CMD_USER_CONFIRM_REPLY, bd.to_vec())
}

/// Link Key Reply: [bd][key16]
pub fn cmd_link_key_reply(bd: [u8; 6], key: [u8; 16]) -> HciCommand {
    let mut v = Vec::with_capacity(22);
    v.extend_from_slice(&bd);
    v.extend_from_slice(&key);
    HciCommand::with_params(CMD_LINK_KEY_REPLY, v)
}

/// Auth requested: [handle u16]
pub fn cmd_auth_requested(handle: u16) -> HciCommand {
    HciCommand::with_params(CMD_AUTH_REQUESTED, handle.to_le_bytes().to_vec())
}

/// Set connection encryption: [handle][enable]
pub fn cmd_set_conn_encrypt(handle: u16, enable: bool) -> HciCommand {
    let mut v = Vec::with_capacity(3);
    v.extend_from_slice(&handle.to_le_bytes());
    v.push(u8::from(enable));
    HciCommand::with_params(CMD_SET_CONN_ENCRYPT, v)
}

// ---- Event parsers ----

/// [status][handle u16][bd][link_type][encrypt]
pub fn parse_connection_complete(params: &[u8]) -> Option<(u8, u16, [u8; 6])> {
    if params.len() < 11 {
        return None;
    }
    let status = params[0];
    let handle = u16::from_le_bytes([params[1], params[2]]);
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[3..9]);
    Some((status, handle, bd))
}

/// [bd][class 3][link_type]
pub fn parse_connection_request(params: &[u8]) -> Option<[u8; 6]> {
    if params.len() < 10 {
        return None;
    }
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[..6]);
    Some(bd)
}

/// [status][handle]
pub fn parse_status_handle(params: &[u8]) -> Option<(u8, u16)> {
    if params.len() < 3 {
        return None;
    }
    let handle = u16::from_le_bytes([params[1], params[2]]);
    Some((params[0], handle))
}

/// [bd] simplesmente
pub fn parse_bd_addr_event(params: &[u8]) -> Option<[u8; 6]> {
    if params.len() < 6 {
        return None;
    }
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[..6]);
    Some(bd)
}

/// Link Key Notification: [bd 6][key 16][key_type]
pub fn parse_link_key_notif(params: &[u8]) -> Option<([u8; 6], [u8; 16], u8)> {
    if params.len() < 23 {
        return None;
    }
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[..6]);
    let mut key = [0u8; 16];
    key.copy_from_slice(&params[6..22]);
    Some((bd, key, params[22]))
}

/// User Confirmation Request: [bd][numeric u32]
pub fn parse_user_confirm_req(params: &[u8]) -> Option<([u8; 6], u32)> {
    if params.len() < 10 {
        return None;
    }
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[..6]);
    let num = u32::from_le_bytes([params[6], params[7], params[8], params[9]]);
    Some((bd, num))
}

/// SSP Complete: [status][bd]
pub fn parse_ssp_complete(params: &[u8]) -> Option<(u8, [u8; 6])> {
    if params.len() < 7 {
        return None;
    }
    let mut bd = [0u8; 6];
    bd.copy_from_slice(&params[1..7]);
    Some((params[0], bd))
}

#[cfg(test)]
mod pairing_codec_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn opcodes_conexao_ssp() {
        assert_eq!(CMD_CREATE_CONNECTION, 0x0405);
        assert_eq!(CMD_ACCEPT_CONNECTION, 0x0409);
        assert_eq!(CMD_PIN_REPLY, 0x040D);
        assert_eq!(CMD_AUTH_REQUESTED, 0x0411);
        assert_eq!(CMD_SET_CONN_ENCRYPT, 0x0413);
        assert_eq!(CMD_WRITE_SSP_MODE, 0x0C56);
        assert_eq!(CMD_IO_CAP_REPLY, 0x042B);
        assert_eq!(CMD_USER_CONFIRM_REPLY, 0x042C);
    }

    #[test]
    fn create_connection_layout() {
        let c = cmd_create_connection([1; 6]);
        assert_eq!(c.params.len(), 13);
        assert_eq!(&c.params[..6], &[1; 6]);
        assert_eq!(&c.params[6..8], &0xCC18u16.to_le_bytes());
    }

    #[test]
    fn pin_reply_layout() {
        let c = cmd_pin_reply([2; 6], "1234");
        assert_eq!(&c.params[..6], &[2; 6]);
        assert_eq!(c.params[6], 4);
        assert_eq!(&c.params[7..11], b"1234");
    }

    #[test]
    fn io_cap_reply_justworks_layout() {
        let bd = [0xAA; 6];
        let c = cmd_io_cap_reply(bd, IO_CAP_NO_INPUT_NO_OUTPUT, 0);
        assert_eq!(&c.params[..6], &bd);
        assert_eq!(c.params[6], 0x03); // NoInputNoOutput
        assert_eq!(c.params[7], 0); // sem OOB
        assert_eq!(c.params[8], 0); // auth mitm off
    }

    #[test]
    fn parse_conn_complete_e_request() {
        // conn complete: status 0, handle 0x20, bd, link 1, enc 0
        let p = [
            0x00, 0x20, 0x00, 1, 2, 3, 4, 5, 6, 0x01, 0x00,
        ];
        let (st, h, bd) = parse_connection_complete(&p).unwrap();
        assert_eq!(st, 0);
        assert_eq!(h, 0x20);
        assert_eq!(bd, [1, 2, 3, 4, 5, 6]);
        assert!(parse_connection_complete(&p[..6]).is_none());

        let req = [9, 8, 7, 6, 5, 4, 0, 0, 0, 0x01];
        assert_eq!(parse_connection_request(&req).unwrap(), [9, 8, 7, 6, 5, 4]);
    }

    #[test]
    fn parse_link_key_e_ssp_e_userconfirm() {
        // link key notif
        let mut p = vec![9; 6];
        p.extend_from_slice(&[0xAB; 16]);
        p.push(0x04); // unauthenticated combination key
        let (bd, key, ty) = parse_link_key_notif(&p).unwrap();
        assert_eq!(bd, [9; 6]);
        assert_eq!(key, [0xAB; 16]);
        assert_eq!(ty, 0x04);

        // user confirm
        let bd = [5; 6];
        let mut uc = bd.to_vec();
        uc.extend_from_slice(&123456u32.to_le_bytes());
        let (b2, num) = parse_user_confirm_req(&uc).unwrap();
        assert_eq!(b2, bd);
        assert_eq!(num, 123456);

        // ssp complete
        let ok = [0x00, 1, 2, 3, 4, 5, 6];
        let (st, bd3) = parse_ssp_complete(&ok).unwrap();
        assert_eq!(st, 0);
        assert_eq!(bd3, [1, 2, 3, 4, 5, 6]);
        // falha
        let fail = [0x06, 1, 2, 3, 4, 5, 6]; // 6 = auth failure
        let (st, _) = parse_ssp_complete(&fail).unwrap();
        assert_eq!(st, 6);
    }
}
