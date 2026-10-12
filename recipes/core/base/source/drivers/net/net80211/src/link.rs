//! Link state machine: scan -> assoc -> associated.
//!
//! Pure logic (no MMIO): drivers call these methods while feeding raw 802.11
//! management frames received on their RX rings.

use alloc::vec::Vec;

/// 802.11 frame-control subtypes for management frames we care about.
pub const STYPE_ASSOC_REQ: u8 = 0;
pub const STYPE_ASSOC_RESP: u8 = 1;
pub const STYPE_BEACON: u8 = 8;

/// Length of the fixed parameters in a beacon frame (timestamp + interval + capab).
const BEACON_FIXED_LEN: usize = 12;
const DOT11_MGMT_HDR_LEN: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkState {
    /// Not associated to any BSS.
    #[default]
    Disconnected,
    Scanning,
    Associating,
    Associated,
}

/// A basic service set discovered by scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bss {
    pub bssid: [u8; 6],
    pub essid: Vec<u8>,
    pub channel: u8,
    pub rssi_dbm: i8,
}

#[derive(Debug, Default)]
pub struct Link {
    state: LinkState,
    scan_results: Vec<Bss>,
    current: Option<Bss>,
}

impl Link {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> LinkState {
        self.state
    }

    pub fn scan_results(&self) -> &[Bss] {
        &self.scan_results
    }

    pub fn current_bss(&self) -> Option<&Bss> {
        self.current.as_ref()
    }

    /// Begin a scan round. Allowed from Disconnected only.
    pub fn start_scan(&mut self) -> Result<(), &'static str> {
        if self.state != LinkState::Disconnected {
            return Err("link: scan only from disconnected");
        }
        self.scan_results.clear();
        self.state = LinkState::Scanning;
        Ok(())
    }

    /// Scan round finished (driver says so after the listen window).
    pub fn finish_scan(&mut self) -> Result<usize, &'static str> {
        if self.state != LinkState::Scanning {
            return Err("link: not scanning");
        }
        self.state = LinkState::Disconnected;
        Ok(self.scan_results.len())
    }

    /// Feed an 802.11 beacon frame. Adds/updates a BSS. Returns the BSSID when new.
    pub fn on_beacon(&mut self, frame: &[u8], rssi_dbm: i8) -> Option<[u8; 6]> {
        if !is_mgmt_subtype(frame, STYPE_BEACON) || frame.len() < DOT11_MGMT_HDR_LEN + BEACON_FIXED_LEN {
            return None;
        }

        let mut bssid = [0u8; 6];
        bssid.copy_from_slice(&frame[16..22]);

        // SSID IE is the first information element after fixed params.
        let ies = &frame[DOT11_MGMT_HDR_LEN + BEACON_FIXED_LEN..];
        let essid = parse_ssid_ie(ies).unwrap_or_default();

        if let Some(existing) = self.scan_results.iter_mut().find(|b| b.bssid == bssid) {
            existing.rssi_dbm = rssi_dbm;
            if !essid.is_empty() {
                existing.essid = essid;
            }
            return None;
        }

        self.scan_results.push(Bss {
            bssid,
            essid,
            channel: 0, // channel vem do DS IE; parser dedicado fica para depois
            rssi_dbm,
        });
        Some(bssid)
    }

    /// Start associating with a known BSS.
    pub fn associate(&mut self, bssid: [u8; 6]) -> Result<&Bss, &'static str> {
        let bss = self
            .scan_results
            .iter()
            .find(|b| b.bssid == bssid)
            .ok_or("link: bssid unknown (scan first)")?
            .clone();
        self.current = Some(bss);
        self.state = LinkState::Associating;
        // Borrow again — current was just set.
        Ok(self.current.as_ref().unwrap())
    }

    /// Handle an association response (`success` = status code 0).
    pub fn on_assoc_response(&mut self, success: bool) {
        if self.state != LinkState::Associating {
            return;
        }
        self.state = if success {
            LinkState::Associated
        } else {
            self.current = None;
            LinkState::Disconnected
        };
    }

    pub fn disassociate(&mut self) {
        self.state = LinkState::Disconnected;
        self.current = None;
    }
}

/// Parse FC and compare against mgmt type + subtype.
pub fn is_mgmt_subtype(frame: &[u8], subtype: u8) -> bool {
    if frame.len() < 2 {
        return false;
    }
    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    let ftype = (fc >> 2) & 0x3;
    let fsubtype = (fc >> 4) & 0xF;
    ftype == 0 && fsubtype == subtype as u16
}

/// Extract the SSID from the first SSID IE (tag 0) in a IE blob.
fn parse_ssid_ie(ies: &[u8]) -> Option<Vec<u8>> {
    let mut off = 0;
    while off + 2 <= ies.len() {
        let (tag, len) = (ies[off], ies[off + 1] as usize);
        if off + 2 + len > ies.len() {
            return None;
        }
        if tag == 0 {
            return Some(ies[off + 2..off + 2 + len].to_vec());
        }
        off += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Build a minimal but well-formed beacon frame.
    fn beacon(bssid: [u8; 6], ssid: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&0x0080u16.to_le_bytes()); // FC: mgmt, subtype 8 (beacon)
        f.extend_from_slice(&0u16.to_le_bytes()); // duration
        f.extend_from_slice(&[0xFF; 6]); // DA = broadcast
        f.extend_from_slice(&bssid); // SA = BSSID
        f.extend_from_slice(&bssid); // BSSID
        f.extend_from_slice(&0u16.to_le_bytes()); // seq
        f.extend_from_slice(&[0u8; 12]); // fixed: ts + interval + capab
        f.push(0); // IE: SSID
        f.push(ssid.len() as u8);
        f.extend_from_slice(ssid);
        f
    }

    #[test]
    fn scan_transitions() {
        let mut l = Link::new();
        assert_eq!(l.state(), LinkState::Disconnected);
        l.start_scan().unwrap();
        assert_eq!(l.state(), LinkState::Scanning);
        // Nao pode scanear de novo no meio do caminho
        assert!(l.start_scan().is_err());
        assert_eq!(l.finish_scan().unwrap(), 0);
        assert_eq!(l.state(), LinkState::Disconnected);
    }

    #[test]
    fn beacon_creates_and_updates_bss() {
        let mut l = Link::new();
        l.start_scan().unwrap();
        let b = beacon([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], b"CafeWifi");
        assert_eq!(l.on_beacon(&b, -42), Some([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]));
        assert_eq!(l.scan_results().len(), 1);
        assert_eq!(l.scan_results()[0].essid, b"CafeWifi");

        // Mesmo BSSID = update (incluindo RSSI), sem duplicar
        assert_eq!(l.on_beacon(&b, -30), None);
        assert_eq!(l.scan_results().len(), 1);
        assert_eq!(l.scan_results()[0].rssi_dbm, -30);

        // Lixo nao vira BSS
        assert_eq!(l.on_beacon(&[0xFF; 10], -10), None);
        assert_eq!(l.scan_results().len(), 1);
    }

    #[test]
    fn assoc_full_cycle_and_failures() {
        let mut l = Link::new();
        // Assoc sem scan = erro
        assert!(l.associate([1; 6]).is_err());

        l.start_scan().unwrap();
        let bssid = [0xAA; 6];
        l.on_beacon(&beacon(bssid, b"Home"), -50);

        // assoc OK
        assert_eq!(l.associate(bssid).unwrap().essid, b"Home");
        assert_eq!(l.state(), LinkState::Associating);
        l.on_assoc_response(true);
        assert_eq!(l.state(), LinkState::Associated);
        assert_eq!(l.current_bss().unwrap().bssid, bssid);

        // disassoc volta ao inicio
        l.disassociate();
        assert_eq!(l.state(), LinkState::Disconnected);
        assert!(l.current_bss().is_none());

        // resposta negada tambem desconecta
        l.link_force_scan_and_learn(bssid);
        l.associate(bssid).unwrap();
        l.on_assoc_response(false);
        assert_eq!(l.state(), LinkState::Disconnected);
    }

    // helper do teste acima (scan direto)
    impl Link {
        #[cfg(test)]
        pub fn link_force_scan_and_learn(&mut self, bssid: [u8; 6]) {
            self.start_scan().unwrap();
            self.on_beacon(&beacon(bssid, b"Home"), -50);
            self.finish_scan().unwrap();
        }
    }

    #[test]
    fn beacon_subtype_check() {
        let b = beacon([0; 6], b"x");
        assert!(is_mgmt_subtype(&b, STYPE_BEACON));
        assert!(!is_mgmt_subtype(&b, STYPE_ASSOC_RESP));
        // Data frame (type 2) nao e mgmt
        let mut d = b.clone();
        d[0] = 0x08;
        d[1] = 0x01;
        assert!(!is_mgmt_subtype(&d, STYPE_BEACON));
        // Curto demais
        assert!(!is_mgmt_subtype(&[0x80], STYPE_BEACON));
    }

    #[test]
    fn ssid_ie_parsing() {
        // IEs: SSID "abc" + Rates 1,2 + outro SSID no fim (invalido, mas parser pega o primeiro)
        let ies = [0, 3, b'a', b'b', b'c', 1, 2, 0x82, 0x84];
        assert_eq!(parse_ssid_ie(&ies).unwrap(), b"abc");
        // Truncado: len maior que o restante
        assert!(parse_ssid_ie(&[0, 10, b'a']).is_none());
        // Sem SSID
        assert!(parse_ssid_ie(&[1, 2, 0x82, 0x84]).is_none());
    }
}
