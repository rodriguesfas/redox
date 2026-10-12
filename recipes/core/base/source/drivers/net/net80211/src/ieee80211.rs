//! IEEE 802.11 frame definitions and constants
//!
//! This module contains the core 802.11 frame structures, constants,
//! and helper functions for parsing and constructing 802.11 frames.

use core::fmt;

/// IEEE 802.11 Frame Control field
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct FrameControl(u16);

impl FrameControl {
    pub const fn new(val: u16) -> Self {
        Self(val)
    }

    pub fn frame_type(&self) -> FrameType {
        FrameType::from_bits_truncate(((self.0 >> 2) & 0x3) as u8)
    }

    pub fn frame_subtype(&self) -> u8 {
        ((self.0 >> 4) & 0xF) as u8
    }

    pub fn to_ds(&self) -> bool {
        (self.0 & (1 << 8)) != 0
    }

    pub fn from_ds(&self) -> bool {
        (self.0 & (1 << 9)) != 0
    }

    pub fn more_frag(&self) -> bool {
        (self.0 & (1 << 10)) != 0
    }

    pub fn retry(&self) -> bool {
        (self.0 & (1 << 11)) != 0
    }

    pub fn pwr_mgt(&self) -> bool {
        (self.0 & (1 << 12)) != 0
    }

    pub fn more_data(&self) -> bool {
        (self.0 & (1 << 13)) != 0
    }

    pub fn protected(&self) -> bool {
        (self.0 & (1 << 14)) != 0
    }

    pub fn order(&self) -> bool {
        (self.0 & (1 << 15)) != 0
    }

    pub fn version(&self) -> u8 {
        (self.0 & 0x3) as u8
    }

    pub fn set_version(&mut self, v: u8) {
        self.0 = (self.0 & !0x3) | ((v & 0x3) as u16);
    }
}

impl Default for FrameControl {
    fn default() -> Self {
        Self(0)
    }
}

impl From<u16> for FrameControl {
    fn from(v: u16) -> Self {
        Self(v)
    }
}

impl From<FrameControl> for u16 {
    fn from(fc: FrameControl) -> Self {
        fc.0
    }
}

/// IEEE 802.11 Frame Type (not a bitflag - these are mutually exclusive)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Management = 0,
    Control = 1,
    Data = 2,
    Extension = 3,
}

impl FrameType {
    pub fn from_bits_truncate(bits: u8) -> Self {
        match bits & 0x3 {
            0 => FrameType::Management,
            1 => FrameType::Control,
            2 => FrameType::Data,
            _ => FrameType::Extension,
        }
    }
}

/// IEEE 802.11 Management Frame Subtypes (4 bits)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MgmtSubtype {
    AssocReq = 0,
    AssocResp = 1,
    ReassocReq = 2,
    ReassocResp = 3,
    ProbeReq = 4,
    ProbeResp = 5,
    TimingAdv = 6,
    Beacon = 8,
    Atim = 9,
    Disassoc = 10,
    Auth = 11,
    Deauth = 12,
    Action = 13,
    ActionNoAck = 14,
}

/// IEEE 802.11 Control Frame Subtypes (4 bits)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CtrlSubtype {
    Wrapper = 7,
    BlockAckReq = 8,
    BlockAck = 9,
    PsPoll = 10,
    Rts = 11,
    Cts = 12,
    Ack = 13,
    CfEnd = 14,
    CfEndAck = 15,
}

/// IEEE 802.11 Data Frame Subtypes (4 bits)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DataSubtype {
    Data = 0,
    DataCfAck = 1,
    DataCfPoll = 2,
    DataCfAckPoll = 3,
    Null = 4,
    CfAck = 5,
    CfPoll = 6,
    CfAckPoll = 7,
    QoSData = 8,
    QoSDataCfAck = 9,
    QoSDataCfPoll = 10,
    QoSDataCfAckPoll = 11,
    QoSNull = 12,
    QoSCfPoll = 14,
    QoSCfAckPoll = 15,
}

/// IEEE 802.11 Sequence Control field
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct SeqCtl(u16);

impl SeqCtl {
    pub const fn new(seq: u16, frag: u16) -> Self {
        Self((seq << 4) | (frag & 0xF))
    }

    pub fn seq(&self) -> u16 {
        self.0 >> 4
    }

    pub fn frag(&self) -> u16 {
        self.0 & 0xF
    }
}

impl From<u16> for SeqCtl {
    fn from(v: u16) -> Self {
        Self(v)
    }
}

impl From<SeqCtl> for u16 {
    fn from(sc: SeqCtl) -> Self {
        sc.0
    }
}

/// 802.11 MAC Address (6 bytes)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    pub const BROADCAST: Self = Self([0xFF; 6]);
    pub const ZERO: Self = Self([0; 6]);

    pub const fn new(addr: [u8; 6]) -> Self {
        Self(addr)
    }

    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }

    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    pub fn is_unicast(&self) -> bool {
        self.0[0] & 0x01 == 0
    }

    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0x01 == 1
    }
}

impl fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
               self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5])
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl From<[u8; 6]> for MacAddr {
    fn from(addr: [u8; 6]) -> Self {
        MacAddr(addr)
    }
}

impl From<MacAddr> for [u8; 6] {
    fn from(mac: MacAddr) -> Self {
        mac.0
    }
}

impl AsRef<[u8]> for MacAddr {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}


/// IEEE 802.11 Element IDs
pub mod elem {
    pub const SSID: u8 = 0;
    pub const SUPPORTED_RATES: u8 = 1;
    pub const FH_PARAMS: u8 = 2;
    pub const DS_PARAMS: u8 = 3;
    pub const CF_PARAMS: u8 = 4;
    pub const TIM: u8 = 5;
    pub const IBSS_PARAMS: u8 = 6;
    pub const COUNTRY: u8 = 7;
    pub const REQUEST: u8 = 10;
    pub const QUIET: u8 = 40;
    pub const IBSS_DFS: u8 = 41;
    pub const CHALLENGE: u8 = 16;
    pub const POWER_CONSTRAINT: u8 = 32;
    pub const CHANNEL_SWITCH: u8 = 37;
    pub const MEASURE_REQUEST: u8 = 38;
    pub const MEASURE_REPORT: u8 = 39;
    pub const POWER_CAPABILITY: u8 = 33;
    pub const TPCC: u8 = 34;
    pub const TPC_REPORT: u8 = 35;
    pub const SUPPORTED_CHANNELS: u8 = 36;
    pub const CHANNEL_SWITCH_TIMING: u8 = 38;
    pub const TPCC_TRANSMIT_POWER: u8 = 39;
    pub const ER_INFO: u8 = 42;
    pub const HT_CAP: u8 = 45;
    pub const QOS_CAP: u8 = 46;
    pub const RSN: u8 = 48;
    pub const EXT_SUPPORTED_RATES: u8 = 50;
    pub const HT_OPERATION: u8 = 61;
    pub const SECONDARY_CH_OFFSET: u8 = 62;
    pub const COEX_2040: u8 = 72;
    pub const OVERLAP_BSS_SCAN: u8 = 74;
    pub const EXT_CAP: u8 = 127;
    pub const VHT_CAP: u8 = 191;
    pub const VHT_OPERATION: u8 = 192;
    pub const EXT_CH_SWITCH_ANN: u8 = 186;
    pub const VHT_TX_POWER_ENVELOPE: u8 = 195;
    pub const HE_CAP: u8 = 255;
    pub const HE_OPERATION: u8 = 255;
    pub const EXTENSION: u8 = 255;
    pub const HE_6GHZ_CAP: u8 = 35;
    pub const MULTI_LINK: u8 = 255;
    pub const RNR: u8 = 24;
    pub const EHT_CAP: u8 = 255;
    pub const EHT_OPERATION: u8 = 255;
}
/// Length of an Ethernet II header (dst+src+ethertype).
pub const ETH_HDR_LEN: usize = 14;
/// Minimum 802.11 MAC header size.
pub const DOT11_HDR_LEN: usize = 24;

/// Convert an 802.11 data frame to Ethernet II.
///
/// `our_mac` is the receiver MAC used for the DA filter; pass broadcast
/// `[0xFF; 6]` to accept anything. Returns `None` for non-data frames,
/// frames not addressed to us, or malformed input.
pub fn dot11_to_ethernet(frame: &[u8], our_mac: [u8; 6], _bssid: [u8; 6]) -> Option<alloc::vec::Vec<u8>> {
    use alloc::vec::Vec;

    if frame.len() < DOT11_HDR_LEN {
        return None;
    }

    let fc = FrameControl::new(u16::from_le_bytes([frame[0], frame[1]]));
    if fc.frame_type() != FrameType::Data {
        return None;
    }

    let to_ds = fc.to_ds();
    let from_ds = fc.from_ds();

    // Address extraction per 802.11 address layout.
    let (src_addr, dst_addr) = match (to_ds, from_ds) {
        (false, false) => (&frame[10..16], &frame[4..10]),  // IBSS: SA=addr2, DA=addr1
        (false, true) => (&frame[16..22], &frame[4..10]),   // FromDS: SA=addr3, DA=addr1
        (true, false) => (&frame[10..16], &frame[16..22]),  // ToDS: SA=addr2, DA=addr3
        (true, true) => (&frame[10..16], &frame[16..22]),   // WDS: SA=addr2(TA), DA=addr3
    };

    if our_mac != [0xFF; 6] && our_mac != dst_addr {
        return None;
    }

    let mut hdr_len = DOT11_HDR_LEN;
    if fc.frame_subtype() & 0x8 != 0 {
        hdr_len += 2; // QoS control
    }
    if fc.0 & (1 << 15) != 0 {
        hdr_len += 4; // HTC
    }
    if to_ds && from_ds {
        hdr_len += 6; // addr4
    }
    if frame.len() < hdr_len {
        return None;
    }

    let payload = &frame[hdr_len..];
    let mut eth = Vec::with_capacity(ETH_HDR_LEN + payload.len());
    eth.extend_from_slice(dst_addr);
    eth.extend_from_slice(src_addr);
    eth.extend_from_slice(&0x0800u16.to_be_bytes()); // EtherType IPv4
    eth.extend_from_slice(payload);
    Some(eth)
}

/// Convert an Ethernet II frame to an 802.11 data frame for STA->AP
/// (ToDS=1: addr1=BSSID, addr2=SA, addr3=DA).
///
/// Only IPv4 (0x0800) is mapped for now.
pub fn ethernet_to_dot11(frame: &[u8], src_mac: [u8; 6], bssid: [u8; 6]) -> Option<alloc::vec::Vec<u8>> {
    use alloc::vec::Vec;

    if frame.len() < ETH_HDR_LEN {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    if ethertype != 0x0800 {
        return None;
    }

    // FC correct for STA->AP: type=Data, ToDS=1 -> 0x0108.
    // NOTE: older in-tree wrappers used 0x0208, which actually sets FromDS;
    // that was a wire-level bug and is NOT replicated here.
    let fc: u16 = 0x0108;
    let payload = &frame[ETH_HDR_LEN..];

    let mut dot11 = Vec::with_capacity(DOT11_HDR_LEN + payload.len());
    dot11.extend_from_slice(&fc.to_le_bytes());
    dot11.extend_from_slice(&0u16.to_le_bytes()); // duration/id
    dot11.extend_from_slice(&bssid); // addr1 = BSSID
    dot11.extend_from_slice(&src_mac); // addr2 = SA
    dot11.extend_from_slice(&frame[0..6]); // addr3 = DA
    dot11.extend_from_slice(&0u16.to_le_bytes()); // seq ctrl
    dot11.extend_from_slice(payload);
    Some(dot11)
}

impl FrameControl {
    /// Raw inner value.
    pub fn bits(&self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod convert_tests {
    use super::*;
    use alloc::vec::Vec;

    fn sample_eth() -> Vec<u8> {
        let mut eth = Vec::new();
        eth.extend_from_slice(&[0xAA; 6]); // dst
        eth.extend_from_slice(&[0xBB; 6]); // src
        eth.extend_from_slice(&0x0800u16.to_be_bytes());
        eth.extend_from_slice(&[1, 2, 3, 4]);
        eth
    }

    #[test]
    fn eth_to_dot11_tods_layout() {
        let eth = sample_eth();
        let out = ethernet_to_dot11(&eth, [0xBB; 6], [0xCC; 6]).unwrap();
        let fc = FrameControl::new(u16::from_le_bytes([out[0], out[1]]));
        assert_eq!(fc.frame_type(), FrameType::Data);
        assert!(fc.to_ds());
        assert!(!fc.from_ds());
        assert_eq!(&out[4..10], &[0xCC; 6]); // BSSID
        assert_eq!(&out[10..16], &[0xBB; 6]); // SA
        assert_eq!(&out[16..22], &[0xAA; 6]); // DA
        assert_eq!(&out[24..], &[1, 2, 3, 4]);
    }

    #[test]
    fn eth_to_dot11_rejects_short_and_non_ipv4() {
        assert!(ethernet_to_dot11(&[0u8; 10], [0; 6], [0; 6]).is_none());
        let mut eth = sample_eth();
        eth[12] = 0x86;
        eth[13] = 0xDD; // IPv6
        assert!(ethernet_to_dot11(&eth, [0; 6], [0; 6]).is_none());
    }

    #[test]
    fn round_trip_tods() {
        let eth = sample_eth();
        let dot11 = ethernet_to_dot11(&eth, [0xBB; 6], [0xCC; 6]).unwrap();
        let back = dot11_to_ethernet(&dot11, [0xAA; 6], [0xCC; 6]).unwrap();
        assert_eq!(&back[0..6], &[0xAA; 6]);
        assert_eq!(&back[6..12], &[0xBB; 6]);
        assert_eq!(&back[14..], &[1, 2, 3, 4]);
    }

    #[test]
    fn from_ds_decode() {
        // AP->STA: FC FromDS=1 -> 0x0208, addr1=DA, addr2=BSSID, addr3=SA.
        let mut f = Vec::new();
        f.extend_from_slice(&0x0208u16.to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&[0xAA; 6]); // DA
        f.extend_from_slice(&[0xCC; 6]); // BSSID
        f.extend_from_slice(&[0xBB; 6]); // SA
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&[7, 7]);

        let eth = dot11_to_ethernet(&f, [0xAA; 6], [0xCC; 6]).unwrap();
        assert_eq!(&eth[0..6], &[0xAA; 6]); // DA
        assert_eq!(&eth[6..12], &[0xBB; 6]); // SA
        assert_eq!(&eth[14..], &[7, 7]);
    }

    #[test]
    fn filter_rejects_other_da() {
        let eth = sample_eth();
        let dot11 = ethernet_to_dot11(&eth, [0xBB; 6], [0xCC; 6]).unwrap();
        assert!(dot11_to_ethernet(&dot11, [0xEE; 6], [0xCC; 6]).is_none());
        // Broadcast filter accepts anything.
        assert!(dot11_to_ethernet(&dot11, [0xFF; 6], [0xCC; 6]).is_some());
    }
}
