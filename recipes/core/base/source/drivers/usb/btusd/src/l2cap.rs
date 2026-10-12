//! L2CAP minimal ACL framing (Bluetooth Core Spec, Vol 4 Part A / Vol 2 Part B).
//!
//! ACL-U packet: [handle(12b)|PB(2b)|BC(2b) LE16][data_len LE16][payload...]

use alloc::vec::Vec;

/// Pacote ACL decodificado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclPacket {
    /// Connection handle (12 bits).
    pub handle: u16,
    /// Packet Boundary flag (2 bits): 0=first non-flush, 1=continuation, 2=first flushable.
    pub pb: u8,
    /// Broadcast flag (2 bits): 0=p2p, 1=active broadcast...
    pub bc: u8,
    pub payload: Vec<u8>,
}

impl AclPacket {
    pub fn new(handle: u16, pb: u8, bc: u8, payload: Vec<u8>) -> Self {
        Self {
            handle: handle & 0x0FFF,
            pb: pb & 0x3,
            bc: bc & 0x3,
            payload,
        }
    }

    /// [handle+pb+bc LE16][len LE16][payload]
    pub fn encode(&self) -> Vec<u8> {
        let hdr = self.handle | ((self.pb as u16) << 12) | ((self.bc as u16) << 14);
        let mut out = Vec::with_capacity(4 + self.payload.len());
        out.extend_from_slice(&hdr.to_le_bytes());
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// Decodifica um pacote; retorna (packet, restante) ou None se truncado.
pub fn decode_acl(input: &[u8]) -> Option<(AclPacket, &[u8])> {
    if input.len() < 4 {
        return None;
    }
    let hdr = u16::from_le_bytes([input[0], input[1]]);
    let len = u16::from_le_bytes([input[2], input[3]]) as usize;
    if input.len() < 4 + len {
        return None;
    }
    let pkt = AclPacket {
        handle: hdr & 0x0FFF,
        pb: ((hdr >> 12) & 0x3) as u8,
        bc: ((hdr >> 14) & 0x3) as u8,
        payload: input[4..4 + len].to_vec(),
    };
    Some((pkt, &input[4 + len..]))
}

#[cfg(test)]
mod l2cap_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn acl_roundtrip() {
        let p = AclPacket::new(0x0ABC, 2, 0, vec![1, 2, 3]);
        let enc = p.encode();
        assert_eq!(&enc[..2], &0x2ABCu16.to_le_bytes()); // handle|pb<<12
        assert_eq!(&enc[2..4], &3u16.to_le_bytes());
        let (dec, rest) = decode_acl(&enc).unwrap();
        assert_eq!(dec, p);
        assert!(rest.is_empty());
    }

    #[test]
    fn acl_rejeita_truncado() {
        assert!(decode_acl(&[0x01]).is_none());
        // len=10 mas so temos 2 de payload
        assert!(decode_acl(&[0x01, 0x00, 10, 0, 1, 2]).is_none());
    }

    #[test]
    fn acl_handle_mascara_12_bits() {
        let p = AclPacket::new(0xFFFF, 3, 3, vec![]);
        assert_eq!(p.handle, 0x0FFF);
        assert_eq!(p.pb, 3);
        assert_eq!(p.bc, 3);
    }

    #[test]
    fn acl_stream_dois_pacotes() {
        let a = AclPacket::new(1, 2, 0, vec![9]).encode();
        let b = AclPacket::new(2, 2, 0, vec![8, 8]).encode();
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let (pa, rest) = decode_acl(&stream).unwrap();
        assert_eq!(pa.handle, 1);
        let (pb, rest) = decode_acl(rest).unwrap();
        assert_eq!(pb.handle, 2);
        assert_eq!(pb.payload, vec![8, 8]);
        assert!(rest.is_empty());
    }
}

// ==== L2CAP signaling (CID 0x0001) + gerente de canais ====

/// CID reserved para comandos de sinalizacao L2CAP.
pub const CID_SIGNALING: u16 = 0x0001;
/// PSM do SDP (Service Discovery Protocol).
pub const PSM_SDP: u16 = 0x0001;

// Codigos de sinalizacao (subset necessario p/ canais de dados)
pub const SIG_CONN_REQ: u8 = 0x02;
pub const SIG_CONN_RSP: u8 = 0x03;
pub const SIG_CONFIG_REQ: u8 = 0x04;
pub const SIG_CONFIG_RSP: u8 = 0x05;
pub const SIG_DISCONN_REQ: u8 = 0x06;
pub const SIG_DISCONN_RSP: u8 = 0x07;

/// Um comando de sinalizacao: [code][ident][len u16][data].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigPacket {
    pub code: u8,
    pub ident: u8,
    pub data: Vec<u8>,
}

impl SigPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.data.len());
        out.push(self.code);
        out.push(self.ident);
        out.extend_from_slice(&(self.data.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    /// Decodifica um payload (payload do ACL em CID 1).
    pub fn decode(input: &[u8]) -> Option<SigPacket> {
        if input.len() < 4 {
            return None;
        }
        let len = u16::from_le_bytes([input[2], input[3]]) as usize;
        if input.len() < 4 + len {
            return None;
        }
        Some(SigPacket {
            code: input[0],
            ident: input[1],
            data: input[4..4 + len].to_vec(),
        })
    }
}

/// Identificadores validos de canais dinamicos L2CAP (0x0040-0xFFFF).
pub const CID_FIRST_DYNAMIC: u16 = 0x0040;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    WaitConnectRsp,
    WaitConfigReq,
    Open,
    Closed,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub scid: u16,
    pub dcid: Option<u16>,
    pub handle: u16, // connection handle do link ACL
    pub psm: u16,
    pub state: ChannelState,
    /// true = canal aceito por nos (remoto abriu); false = nos abrimos
    pub inbound: bool,
}

impl Channel {
    /// Nosso cid local (o que aparece como destino nos pacotes recebidos).
    pub fn local_cid(&self) -> u16 {
        if self.inbound { self.dcid.unwrap_or(0) } else { self.scid }
    }
    /// Cid do lado remoto (para quem enviamos).
    pub fn remote_cid(&self) -> u16 {
        if self.inbound { self.scid } else { self.dcid.unwrap_or(0) }
    }

    /// usado interno de encode_for p/ chave de busca.
    fn local_cid_by_outbound_key(&self, dcid: u16) -> Option<u16> {
        // compatibilidade com chamadas antigas: encode_for buscava por ch.dcid==Some(dcid)
        if !self.inbound && self.dcid == Some(dcid) {
            Some(dcid)
        } else if self.inbound && self.dcid == Some(dcid) {
            Some(dcid) // inbound: encode_for by our local dcid manda pro lado remoto (scid)
        } else {
            None
        }
    }
}

#[derive(Debug, Default)]
pub struct ChannelTable {
    channels: Vec<Channel>,
    next_scid: u16,
}

impl ChannelTable {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            next_scid: CID_FIRST_DYNAMIC,
        }
    }

    /// Aloca SCID local e emite L2CAP Connection Request (SIG no CID 1).
    pub fn open(&mut self, handle: u16, psm: u16, ident: u8) -> (Channel, SigPacket) {
        let scid = self.next_scid;
        self.next_scid = self.next_scid.wrapping_add(1);
        if self.next_scid < CID_FIRST_DYNAMIC {
            self.next_scid = CID_FIRST_DYNAMIC;
        }
        let ch = Channel {
            scid,
            dcid: None,
            handle,
            psm,
            state: ChannelState::WaitConnectRsp,
            inbound: false,
        };
        self.channels.push(ch.clone());

        let mut data = Vec::with_capacity(4);
        data.extend_from_slice(&psm.to_le_bytes());
        data.extend_from_slice(&scid.to_le_bytes());
        let sig = SigPacket {
            code: SIG_CONN_REQ,
            ident,
            data,
        };
        (ch, sig)
    }

    /// Processa o Connection Response do remoto; Some(dcid) se aceito.
    pub fn on_conn_rsp(&mut self, scid: u16, dcid: u16, result: u16, ident_rsp: u8) -> Result<(u16, SigPacket), &'static str> {
        let ch = self
            .channels
            .iter_mut()
            .find(|c| c.scid == scid)
            .ok_or("l2cap: resposta p/ canal desconhecido")?;
        if result != 0 || dcid == 0 {
            ch.state = ChannelState::Closed;
            return Err("l2cap: conexao recusada pelo remoto");
        }
        ch.dcid = Some(dcid);
        ch.state = ChannelState::WaitConfigReq;

        // Responde com Configuration Request minima (sem opcoes)
        let mut data = Vec::with_capacity(4);
        data.extend_from_slice(&dcid.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // flags
        Ok((
            dcid,
            SigPacket {
                code: SIG_CONFIG_REQ,
                ident: ident_rsp.wrapping_add(1),
                data,
            },
        ))
    }

    /// Configuration Response do remoto confirma -> canal aberto.
    pub fn on_config_rsp(&mut self, dcid: u16, result: u16) -> Result<(), &'static str> {
        let ch = self
            .channels
            .iter_mut()
            .find(|c| c.dcid == Some(dcid))
            .ok_or("l2cap: config rsp de canal desconhecido")?;
        if result != 0 {
            ch.state = ChannelState::Closed;
            return Err("l2cap: config falhou");
        }
        ch.state = ChannelState::Open;
        Ok(())
    }

    /// Connection Request de ENTRADA (o remoto abre canal conosco).
    /// Aceita so PSM_SDP (nosso servidor SDP). Retorna a ConnRsp p/ enviar.
    pub fn on_conn_req(&mut self, handle: u16, scid_remote: u16, psm: u16, _ident: u8) -> (bool, SigPacket) {
        let (accepted, dcid_local, result) = if psm == PSM_SDP {
            let dcid = self.next_scid;
            self.next_scid = self.next_scid.wrapping_add(1);
            if self.next_scid < CID_FIRST_DYNAMIC {
                self.next_scid = CID_FIRST_DYNAMIC;
            }
            (true, dcid, 0u16)
        } else {
            (false, 0, 2u16) // PSM not supported
        };

        if accepted {
            self.channels.push(Channel {
                scid: scid_remote,
                dcid: Some(dcid_local),
                handle,
                psm,
                state: ChannelState::WaitConfigReq,
                inbound: true,
            });
        }

        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&dcid_local.to_le_bytes());
        data.extend_from_slice(&scid_remote.to_le_bytes());
        data.extend_from_slice(&result.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // status
        (accepted, SigPacket {
            code: SIG_CONN_RSP,
            ident: _ident,
            data,
        })
    }

    /// Configuration Request de entrada: aceitamos qualquer config minima.
    /// Retorna (Some ConfigRsp, marca Open) ou (None) se canal desconhecido.
    pub fn on_config_req(&mut self, _ident: u8, dcid: u16) -> Option<SigPacket> {
        let ch = self
            .channels
            .iter_mut()
            .find(|c| c.dcid == Some(dcid))?;
        ch.state = ChannelState::Open;
        // ConfigRsp: [scid = scid do remoto? na vdd: SCID que pedimos = nosso DCID
        //   na verdade o payload e [SCID(target=source do pedido) etc]; usamos dcid do remoto.
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&ch.scid.to_le_bytes()); // Source CID (deles)
        data.extend_from_slice(&0u16.to_le_bytes()); // flags
        data.extend_from_slice(&0u16.to_le_bytes()); // result OK
        Some(SigPacket {
            code: SIG_CONFIG_RSP,
            ident: _ident,
            data,
        })
    }

    /// Canal aberto pelo cid local (o nosso, ondem o remoto endereca).
    pub fn channel_by_local_cid(&self, cid: u16) -> Option<&Channel> {
        self.channels
            .iter()
            .find(|c| c.local_cid() == cid && c.state == ChannelState::Open)
    }

    fn local_cid_matches(&self, dcid: u16) -> bool {
        self.channels.iter().any(|c| c.local_cid() == dcid)
    }

    /// Empacota dados de um canal aberto num ACL (PB=2 first non-flushable).
    pub fn encode_for(&self, dcid: u16, payload: &[u8]) -> Result<AclPacket, &'static str> {
        let ch = self
            .channels
            .iter()
            .find(|c| c.local_cid_by_outbound_key(dcid) == Some(dcid) && c.state == ChannelState::Open)
            .ok_or("l2cap: canal nao esta aberto")?;
        let remote = ch.remote_cid();
        let mut acl_payload = Vec::with_capacity(4 + payload.len());
        acl_payload.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        acl_payload.extend_from_slice(&remote.to_le_bytes()); // DCID do REmoto
        acl_payload.extend_from_slice(payload);
        Ok(AclPacket::new(ch.handle, 2, 0, acl_payload))
    }

    /// Decodifica ACL recebido e o roteia p/ o canal aberto certo.
    pub fn route(&self, pkt: &AclPacket) -> Result<(u16, Vec<u8>), &'static str> {
        if pkt.payload.len() < 4 {
            return Err("l2cap: acl payload truncado");
        }
        let dcid = u16::from_le_bytes([pkt.payload[2], pkt.payload[3]]);
        let _len = u16::from_le_bytes([pkt.payload[0], pkt.payload[1]]);
        let ch = self
            .channels
            .iter()
            .find(|c| c.local_cid() == dcid && c.state == ChannelState::Open)
            .ok_or("l2cap: acl p/ canal nao aberto")?;
        Ok((ch.scid, pkt.payload[4..].to_vec()))
    }
}

#[cfg(test)]
mod l2cap_sig_tests {
    use super::*;
    use alloc::vec;

    const H: u16 = 0x20;

    #[test]
    fn sig_roundtrip() {
        let sig = SigPacket {
            code: SIG_CONN_REQ,
            ident: 9,
            data: vec![0x01, 0x00, 0x40, 0x00], // PSM SDP, SCID 0x40
        };
        let enc = sig.encode();
        assert_eq!(enc[0], 0x02);
        assert_eq!(enc[1], 9);
        let len = u16::from_le_bytes([enc[2], enc[3]]);
        assert_eq!(len, 4);
        let dec = SigPacket::decode(&enc).unwrap();
        assert_eq!(dec, sig);
        // truncados
        assert!(SigPacket::decode(&enc[..5]).is_none());
    }

    #[test]
    fn canal_abre_fecha_to_config() {
        let mut t = ChannelTable::new();
        let (ch, sig) = t.open(H, PSM_SDP, 1);
        assert_eq!(ch.scid, CID_FIRST_DYNAMIC);
        assert_eq!(sig.code, SIG_CONN_REQ);
        assert_eq!(u16::from_le_bytes([sig.data[0], sig.data[1]]), PSM_SDP);

        // remoto aceita com DCID 0x0050 -> config req sai
        let (dcid, cfg) = t.on_conn_rsp(ch.scid, 0x0050, 0, 7).unwrap();
        assert_eq!(dcid, 0x0050);
        assert_eq!(cfg.code, SIG_CONFIG_REQ);
        assert_eq!(u16::from_le_bytes([cfg.data[0], cfg.data[1]]), 0x0050);

        // config rsp ok -> open; dados saem validos
        t.on_config_rsp(0x0050, 0).unwrap();
        let acl = t.encode_for(0x0050, b"ping").unwrap();
        assert_eq!(acl.handle, H);
        assert_eq!(acl.pb, 2);
        // L2CAP b-frame: [len][dcid][data]
        assert_eq!(&acl.payload[..2], &4u16.to_le_bytes());
        assert_eq!(&acl.payload[2..4], &0x0050u16.to_le_bytes());
        assert_eq!(&acl.payload[4..], b"ping");

        // roteamento de dados vindos DO REMOTO: DCID= nosso cid local (0x40)
        let mut inbound = Vec::new();
        inbound.extend_from_slice(&4u16.to_le_bytes());
        inbound.extend_from_slice(&CID_FIRST_DYNAMIC.to_le_bytes());
        inbound.extend_from_slice(b"ping");
        let (scid2, data) = t.route(&AclPacket::new(H, 2, 0, inbound)).unwrap();
        assert_eq!(scid2, CID_FIRST_DYNAMIC);
        assert_eq!(data, b"ping");
    }

    #[test]
    fn canais_negados_e_psm_fechado() {
        let mut t = ChannelTable::new();
        let (ch, _) = t.open(H, PSM_SDP, 1);
        // resposta com result != 0 = recusada
        assert!(t.on_conn_rsp(ch.scid, 0, 2, 1).is_err()); // result 2 = psm nao suportado
        assert_eq!(
            t.channels[0].state,
            ChannelState::Closed
        );
        // dados em canal fechado => erro
        assert!(t.encode_for(0x0050, b"x").is_err());
    }

    #[test]
    fn acl_route_rejeita_canais_nao_abertos() {
        let mut t = ChannelTable::new();
        t.open(H, PSM_SDP, 1);
        // ainda nao ha conn rsp; ACL que finge dcid 0x40 deve falhar
        let payload = [2u8, 0, 0x40, 0, 9, 9];
        let p = AclPacket::new(H, 2, 0, payload.to_vec());
        assert!(t.route(&p).is_err());
    }
}
