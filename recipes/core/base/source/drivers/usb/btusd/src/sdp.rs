//! SDP (Service Discovery Protocol) subset for btusd.
//!
//! Implementa: codec de Data Elements (big-endian), PDU encode/decode,
//! registry de Service Records, e handlers para Service Search / Service
//! Attribute / Service Search Attribute. RFCOMM para fora (clientes usam).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

// ================= Data Elements =================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataElement {
    Nil,
    U8(u8),
    U16(u16),
    U32(u32),
    Uuid16(u16),
    Uuid128([u8; 16]),
    Bool(bool),
    Text(String),
    Seq(Vec<DataElement>),
}

/// size index: 0=1,1=2,2=4,3=8,4=16 bytes; 5=u8 len, 6=u16 len, 7=u32 len
fn header(ty: u8, sz: u8) -> u8 {
    (ty << 3) | (sz & 7)
}

impl DataElement {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Nil => out.push(header(0, 0)),
            Self::U8(v) => {
                out.push(header(1, 0));
                out.push(*v);
            }
            Self::U16(v) => {
                out.push(header(1, 1));
                out.extend_from_slice(&v.to_be_bytes());
            }
            Self::U32(v) => {
                out.push(header(1, 2));
                out.extend_from_slice(&v.to_be_bytes());
            }
            Self::Uuid16(v) => {
                out.push(header(3, 1));
                out.extend_from_slice(&v.to_be_bytes());
            }
            Self::Uuid128(v) => {
                out.push(header(3, 4));
                out.extend_from_slice(v);
            }
            Self::Bool(v) => {
                out.push(header(5, 0));
                out.push(u8::from(*v));
            }
            Self::Text(s) => {
                let b = s.as_bytes();
                match b.len() {
                    n if n <= 255 => {
                        out.push(header(4, 5));
                        out.push(n as u8);
                    }
                    n if n <= 65535 => {
                        out.push(header(4, 6));
                        out.extend_from_slice(&(n as u16).to_be_bytes());
                    }
                    _ => return out, // truncate unsupported
                }
                out.extend_from_slice(b);
            }
            Self::Seq(items) => {
                let body: Vec<u8> = items.iter().flat_map(|i| i.encode()).collect();
                match body.len() {
                    n if n <= 255 => {
                        out.push(header(6, 5));
                        out.push(n as u8);
                    }
                    n if n <= 65535 => {
                        out.push(header(6, 6));
                        out.extend_from_slice(&(n as u16).to_be_bytes());
                    }
                    _ => return out,
                }
                out.extend_from_slice(&body);
            }
        }
        out
    }

    /// (elemento, resto do buffer) ou None se truncado.
    pub fn decode(input: &[u8]) -> Option<(Self, &[u8])> {
        let (&hdr, rest) = input.split_first()?;
        let ty = hdr >> 3;
        let sz = hdr & 0x07;
        // Nil: sem payload (0 bytes depois do header).
        if ty == 0 {
            return Some((Self::Nil, rest));
        }
        let fixed: Option<usize> = match sz {
            0 => Some(1),
            1 => Some(2),
            2 => Some(4),
            3 => Some(8),
            4 => Some(16),
            _ => None,
        };

        if let Some(n) = fixed {
            if rest.len() < n {
                return None;
            }
            let (got, rest2) = rest.split_at(n);
            let el = match (ty, sz) {
                (0, 0) => Self::Nil,
                (1, 0) => Self::U8(got[0]),
                (1, 1) => Self::U16(u16::from_be_bytes([got[0], got[1]])),
                (1, 2) => Self::U32(u32::from_be_bytes([got[0], got[1], got[2], got[3]])),
                (3, 1) => Self::Uuid16(u16::from_be_bytes([got[0], got[1]])),
                (3, 4) => {
                    let mut a = [0u8; 16];
                    a.copy_from_slice(got);
                    Self::Uuid128(a)
                }
                (5, 0) => Self::Bool(got[0] != 0),
                _ => return None,
            };
            return Some((el, rest2));
        }

        // variable size: len despues do header
        if sz == 5 || sz == 6 {
            let lbytes = if sz == 5 { 1 } else { 2 };
            if rest.len() < lbytes {
                return None;
            }
            let n = if sz == 5 {
                rest[0] as usize
            } else {
                u16::from_be_bytes([rest[0], rest[1]]) as usize
            };
            if rest.len() < lbytes + n {
                return None;
            }
            let (got, rest2) = rest[lbytes..].split_at(n);
            let el = match ty {
                4 => Self::Text(core::str::from_utf8(got).ok()?.into()),
                6 => {
                    let mut items = Vec::new();
                    let mut cursor = got;
                    while !cursor.is_empty() {
                        let (e, r) = Self::decode(cursor)?;
                        items.push(e);
                        cursor = r;
                    }
                    Self::Seq(items)
                }
                _ => return None,
            };
            return Some((el, rest2));
        }
        None
    }
}

// ================= PDU =================

pub const PDU_ERROR: u8 = 0x01;
pub const PDU_SVC_SEARCH_REQ: u8 = 0x02;
pub const PDU_SVC_SEARCH_RSP: u8 = 0x03;
pub const PDU_SVC_ATTR_REQ: u8 = 0x04;
pub const PDU_SVC_ATTR_RSP: u8 = 0x05;
pub const PDU_SVC_SEARCH_ATTR_REQ: u8 = 0x06;
pub const PDU_SVC_SEARCH_ATTR_RSP: u8 = 0x07;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdu {
    pub id: u8,
    pub tx_id: u16,
    pub params: Vec<u8>,
}

impl Pdu {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.params.len());
        out.push(self.id);
        out.extend_from_slice(&self.tx_id.to_be_bytes());
        out.extend_from_slice(&(self.params.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.params);
        out
    }

    pub fn decode(input: &[u8]) -> Option<(Self, &[u8])> {
        if input.len() < 5 {
            return None;
        }
        let len = u16::from_be_bytes([input[3], input[4]]) as usize;
        if input.len() < 5 + len {
            return None;
        }
        Some((
            Pdu {
                id: input[0],
                tx_id: u16::from_be_bytes([input[1], input[2]]),
                params: input[5..5 + len].to_vec(),
            },
            &input[5 + len..],
        ))
    }
}

// ================= Service Record + Registry =================

/// Attr IDs usados (SDP Part E, "Universal Attributes").
pub const ATTR_SERVICE_RECORD_HANDLE: u16 = 0x0000;
pub const ATTR_SERVICE_CLASS_ID_LIST: u16 = 0x0001;
pub const ATTR_PROTOCOL_DESCRIPTOR_LIST: u16 = 0x0004;
pub const ATTR_SERVICE_NAME: u16 = 0x0100;

#[derive(Debug, Clone, Default)]
pub struct ServiceRecord {
    pub handle: u32,
    pub attrs: Vec<(u16, DataElement)>,
}

impl ServiceRecord {
    pub fn attr(&self, id: u16) -> Option<&DataElement> {
        self.attrs.iter().find(|(i, _)| *i == id).map(|(_, v)| v)
    }
}

#[derive(Debug, Default)]
pub struct SdpRegistry {
    records: BTreeMap<u32, ServiceRecord>,
}

impl SdpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, rec: ServiceRecord) {
        self.records.insert(rec.handle, rec);
    }

    pub fn get(&self, handle: u32) -> Option<&ServiceRecord> {
        self.records.get(&handle)
    }

    pub fn count(&self) -> usize {
        self.records.len()
    }

    pub fn handles_matching(&self, uuids: &[u16]) -> Vec<u32> {
        self.records
            .iter()
            .filter(|(_, r)| {
                uuids.iter().all(|want| {
                    if let Some(DataElement::Seq(list)) = r.attr(ATTR_SERVICE_CLASS_ID_LIST) {
                        list.iter()
                            .any(|e| matches!(e, DataElement::Uuid16(u) if u == want))
                    } else {
                        false
                    }
                })
            })
            .map(|(h, _)| *h)
            .collect()
    }

    /// Marshal: SEQ { SEQ { attrid, attrval } ... } por registro (busca por atributo)
    pub fn marshal_record(record: &ServiceRecord) -> DataElement {
        DataElement::Seq(
            record
                .attrs
                .iter()
                .flat_map(|(id, v)| Vec::from([DataElement::U16(*id), v.clone()]))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn delems_roundtrip() {
        let elems = vec![
            DataElement::Nil,
            DataElement::U8(0x42),
            DataElement::U16(0x1101),
            DataElement::U32(0x0001_0000),
            DataElement::Uuid16(0x1101),
            DataElement::Bool(true),
            DataElement::Text("Serial".into()),
            DataElement::Seq(vec![DataElement::Uuid16(0x1101), DataElement::U16(3)]),
        ];
        for el in elems {
            let enc = el.encode();
            let (dec, rest) = DataElement::decode(&enc).unwrap();
            assert_eq!(dec, el);
            assert!(rest.is_empty());
        }
    }

    #[test]
    fn delems_nested_seq_and_generic() {
        let nested = DataElement::Seq(vec![DataElement::Uuid128([0xA; 16])]);
        let mut enc = nested.encode();
        enc.extend_from_slice(&[0x00]); // padding
        let (dec, rest) = DataElement::decode(&enc).unwrap();
        assert_eq!(dec, nested);
        assert_eq!(rest.len(), 1);
    }

    #[test]
    fn delem_truncated_fails() {
        assert!(DataElement::decode(&[0x1C, 0x01]).is_none()); // uuid128 short
        assert!(DataElement::decode(&[0x26, 0x00, 0x08, 0x41]).is_none()); // text var16 short
    }

    #[test]
    fn registry_search_by_uuid() {
        let mut reg = SdpRegistry::new();
        reg.register(ServiceRecord {
            handle: 0x10000,
            attrs: vec![
                (ATTR_SERVICE_RECORD_HANDLE, DataElement::U32(0x10000)),
                (
                    ATTR_SERVICE_CLASS_ID_LIST,
                    DataElement::Seq(vec![DataElement::Uuid16(0x1101)]), // Serial Port
                ),
                (ATTR_SERVICE_NAME, DataElement::Text("aios-serial".into())),
            ],
        });
        reg.register(ServiceRecord {
            handle: 0x10001,
            attrs: vec![(
                ATTR_SERVICE_CLASS_ID_LIST,
                DataElement::Seq(vec![DataElement::Uuid16(0x111E)]), // HFP
            )],
        });

        let found = reg.handles_matching(&[0x1101]);
        assert_eq!(found, vec![0x10000]);
        assert!(reg.handles_matching(&[0x9999]).is_empty());
        assert_eq!(reg.handles_matching(&[0x111E]), vec![0x10001]);
        assert_eq!(reg.count(), 2);
    }

    #[test]
    fn pdu_roundtrip() {
        let p = Pdu { id: PDU_SVC_SEARCH_REQ, tx_id: 0x1234, params: vec![1, 2] };
        let enc = p.encode();
        assert_eq!(&enc[..5], &[0x02, 0x12, 0x34, 0x00, 0x02]);
        let (dec, rest) = Pdu::decode(&enc).unwrap();
        assert_eq!(dec, p);
        assert!(rest.is_empty());
        assert!(Pdu::decode(&enc[..4]).is_none());
    }

    #[test]
    fn marshal_record_tem_uuid_e_nome() {
        let rec = ServiceRecord {
            handle: 7,
            attrs: vec![
                (ATTR_SERVICE_CLASS_ID_LIST, DataElement::Seq(vec![DataElement::Uuid16(0x1101)])),
                (ATTR_SERVICE_NAME, DataElement::Text("aios-serial".into())),
            ],
        };
        let m = SdpRegistry::marshal_record(&rec);
        let enc = m.encode();
        // contem UUID no corpo
        let uuid16_enc = DataElement::Uuid16(0x1101).encode();
        assert!(enc.windows(uuid16_enc.len()).any(|w| w == uuid16_enc));
    }
}
