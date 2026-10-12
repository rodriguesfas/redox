//! SDP server: responde buscas/atributos usando o SdpRegistry.

use alloc::vec::Vec;

use super::sdp::*;

#[derive(Debug, Default)]
pub struct SdpServer {
    pub registry: SdpRegistry,
}

impl SdpServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Poll: recebe PDU, devolve PDU resposta (ou Error).
    pub fn handle(&mut self, pkt: &[u8]) -> Option<Vec<u8>> {
        let (pdu, _rest) = Pdu::decode(pkt)?;
        Some(self.dispatch(pdu).encode())
    }

    fn dispatch(&mut self, pdu: Pdu) -> Pdu {
        match pdu.id {
            PDU_SVC_SEARCH_REQ => self.svc_search(pdu),
            PDU_SVC_ATTR_REQ => self.svc_attr(pdu),
            PDU_SVC_SEARCH_ATTR_REQ => self.svc_search_attr(pdu),
            _ => Pdu {
                id: PDU_ERROR,
                tx_id: pdu.tx_id,
                params: alloc::vec![0x00, 0x01], // unsupported version
            },
        }
    }

    /// Service Search: [pattern=Seq(Uuid16/Uuid32/Uuid128)][max u16][continuation]
    fn svc_search(&self, pdu: Pdu) -> Pdu {
        let Some((pattern_el, rest)) = DataElement::decode(&pdu.params) else {
            return self.error(pdu, 0x03); // invalid syntax
        };
        let DataElement::Seq(items) = pattern_el else {
            return self.error(pdu, 0x03);
        };
        let uuids: Vec<u16> = items
            .iter()
            .filter_map(|e| match e {
                DataElement::Uuid16(u) => Some(*u),
                _ => None,
            })
            .collect();
        if uuids.is_empty() {
            return self.error(pdu, 0x03);
        }
        if rest.len() < 2 {
            return self.error(pdu, 0x03);
        }
        let _max = u16::from_be_bytes([rest[0], rest[1]]);

        let handles = self.registry.handles_matching(&uuids);
        let mut params = Vec::new();
        params.extend_from_slice(&(handles.len() as u16).to_be_bytes());
        params.extend_from_slice(&(handles.len() as u16).to_be_bytes());
        for h in handles {
            params.extend_from_slice(&h.to_be_bytes());
        }
        params.push(0x00); // continuation state: none
        Pdu {
            id: PDU_SVC_SEARCH_RSP,
            tx_id: pdu.tx_id,
            params,
        }
    }

    /// Service Attribute: [handle u32][max u16][attrids Seq of U16][continuation]
    fn svc_attr(&self, pdu: Pdu) -> Pdu {
        if pdu.params.len() < 6 {
            return self.error(pdu, 0x03);
        }
        let handle = u32::from_be_bytes([
            pdu.params[0],
            pdu.params[1],
            pdu.params[2],
            pdu.params[3],
        ]);
        let _max = u16::from_be_bytes([pdu.params[4], pdu.params[5]]);
        let Some(rec) = self.registry.get(handle) else {
            return self.error(pdu, 0x04); // invalid service record handle
        };

        let list = SdpRegistry::marshal_record(rec);
        let body = list.encode();
        let mut params = Vec::new();
        params.extend_from_slice(&(body.len() as u16).to_be_bytes());
        params.extend_from_slice(&body);
        params.push(0x00);
        Pdu {
            id: PDU_SVC_ATTR_RSP,
            tx_id: pdu.tx_id,
            params,
        }
    }

    /// Service Search Attribute: [pattern Seq][max u16][attr Seq][continuation]
    fn svc_search_attr(&self, pdu: Pdu) -> Pdu {
        let Some((pattern_el, rest)) = DataElement::decode(&pdu.params) else {
            return self.error(pdu, 0x03);
        };
        let DataElement::Seq(items) = pattern_el else {
            return self.error(pdu, 0x03);
        };
        let uuids: Vec<u16> = items
            .iter()
            .filter_map(|e| match e {
                DataElement::Uuid16(u) => Some(*u),
                _ => None,
            })
            .collect();
        let handles = self.registry.handles_matching(&uuids);

        let mut out_seqs = Vec::new();
        for h in handles {
            if let Some(rec) = self.registry.get(h) {
                out_seqs.push(SdpRegistry::marshal_record(rec));
            }
        }
        let outer = DataElement::Seq(out_seqs);
        let body = outer.encode();
        let mut params = Vec::new();
        params.extend_from_slice(&(body.len() as u16).to_be_bytes());
        params.extend_from_slice(&body);
        params.push(0x00);
        Pdu {
            id: PDU_SVC_SEARCH_ATTR_RSP,
            tx_id: pdu.tx_id,
            params,
        }
    }

    fn error(&self, pdu: Pdu, code: u16) -> Pdu {
        Pdu {
            id: PDU_ERROR,
            tx_id: pdu.tx_id,
            params: alloc::vec![
                (code >> 8) as u8,
                (code & 0xFF) as u8
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn setup_server_with_serial() -> SdpServer {
        let mut srv = SdpServer::new();
        srv.registry.register(ServiceRecord {
            handle: 0x10000,
            attrs: vec![
                (ATTR_SERVICE_RECORD_HANDLE, DataElement::U32(0x10000)),
                (
                    ATTR_SERVICE_CLASS_ID_LIST,
                    DataElement::Seq(vec![DataElement::Uuid16(0x1101)]),
                ),
                (ATTR_SERVICE_NAME, DataElement::Text("aios-serial".into())),
            ],
        });
        srv
    }

    fn attr_range_all() -> DataElement {
        DataElement::Seq(vec![DataElement::U32(0x0000_FFFF)]) // 0x0000-0xFFFF
    }

    #[test]
    fn svc_search_by_uuid_serial() {
        let mut srv = setup_server_with_serial();
        // ServiceSearchRequest params: [Seq(Uuid16 0x1101)][max=10][cont=0]
        let mut params = DataElement::Seq(vec![DataElement::Uuid16(0x1101)]).encode();
        params.extend_from_slice(&10u16.to_be_bytes());
        params.push(0x00);
        let req = Pdu {
            id: PDU_SVC_SEARCH_REQ,
            tx_id: 0xABCD,
            params,
        };
        let rsp = srv.handle(&req.encode()).unwrap();
        let (rpdu, _) = Pdu::decode(&rsp).unwrap();
        assert_eq!(rpdu.id, PDU_SVC_SEARCH_RSP);
        assert_eq!(rpdu.tx_id, 0xABCD);
        // total=1, current=1, handle 0x00010000, cont=0
        assert_eq!(u16::from_be_bytes([rpdu.params[0], rpdu.params[1]]), 1);
        assert_eq!(u16::from_be_bytes([rpdu.params[2], rpdu.params[3]]), 1);
        assert_eq!(
            u32::from_be_bytes([
                rpdu.params[4],
                rpdu.params[5],
                rpdu.params[6],
                rpdu.params[7]
            ]),
            0x10000
        );
    }

    #[test]
    fn svc_attr_returns_record() {
        let mut srv = setup_server_with_serial();
        let mut params = Vec::new();
        params.extend_from_slice(&0x10000u32.to_be_bytes());
        params.extend_from_slice(&100u16.to_be_bytes());
        params.extend_from_slice(&attr_range_all().encode());
        params.push(0x00);
        let req = Pdu {
            id: PDU_SVC_ATTR_REQ,
            tx_id: 1,
            params,
        };
        let rsp = srv.handle(&req.encode()).unwrap();
        let (rpdu, _) = Pdu::decode(&rsp).unwrap();
        assert_eq!(rpdu.id, PDU_SVC_ATTR_RSP);
        // contem UUID 0x1101 na lista
        let uuid_enc = DataElement::Uuid16(0x1101).encode();
        assert!(rsp.windows(uuid_enc.len()).any(|w| w == uuid_enc));
        // e o nome do servico
        let name_enc = DataElement::Text("aios-serial".into()).encode();
        assert!(rsp.windows(name_enc.len()).any(|w| w == name_enc));
    }

    #[test]
    fn svc_search_attr_full() {
        let mut srv = setup_server_with_serial();
        // ServiceSearchAttributeRequest: [pattern Seq(Uuid16)][max][attr-range Seq][cont]
        let mut params = DataElement::Seq(vec![DataElement::Uuid16(0x1101)]).encode();
        params.extend_from_slice(&50u16.to_be_bytes());
        params.extend_from_slice(&attr_range_all().encode());
        params.push(0x00);
        let req = Pdu {
            id: PDU_SVC_SEARCH_ATTR_REQ,
            tx_id: 0x9999,
            params,
        };
        let rsp = srv.handle(&req.encode()).unwrap();
        let (rpdu, _) = Pdu::decode(&rsp).unwrap();
        assert_eq!(rpdu.id, PDU_SVC_SEARCH_ATTR_RSP);
        let uuid_enc = DataElement::Uuid16(0x1101).encode();
        assert!(rsp.windows(uuid_enc.len()).any(|w| w == uuid_enc));
    }

    #[test]
    fn meta_pdu_error_on_unknwon() {
        let mut srv = setup_server_with_serial();
        let req = Pdu {
            id: 0xFE,
            tx_id: 1,
            params: vec![],
        };
        let rsp = srv.handle(&req.encode()).unwrap();
        let (rpdu, _) = Pdu::decode(&rsp).unwrap();
        assert_eq!(rpdu.id, PDU_ERROR);
    }
}
