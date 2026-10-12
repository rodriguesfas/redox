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
