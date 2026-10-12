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
