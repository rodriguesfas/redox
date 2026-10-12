//! Regulatory domain support
//!
//! Parses and enforces wireless regulatory domains per country code.
//! Based on wireless-regdb and CRDA.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Regulatory domain flags
bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RegFlags: u16 {
        const NO_OFDM = 1u16 << 0;
        const NO_CCK = 1u16 << 1;
        const NO_INDOOR = 1u16 << 2;
        const NO_OUTDOOR = 1u16 << 3;
        const DFS = 1u16 << 4;
        const P2P_CONCURRENT = 1u16 << 5;
        const GO_CONCURRENT = 1u16 << 6;
        const NO_IR = 1u16 << 7;
        const AUTO_BW = 1u16 << 8;
        const NO_HT40 = 1u16 << 8;
        const NO_80MHZ = 1u16 << 9;
        const NO_160MHZ = 1u16 << 9;
        const NO_HE = 1u16 << 10;
    }
}

/// Regulatory rule for a frequency range
#[derive(Clone, Debug)]
pub struct RegRule {
    pub freq_start: u32,      // MHz
    pub freq_end: u32,        // MHz
    pub max_bw: u32,          // kHz
    pub max_power: i8,        // dBm
    pub flags: RegFlags,
    pub dfs_cac_time: u16,    // DFS CAC time in ms
}

impl RegRule {
    pub fn new(freq_start: u32, freq_end: u32, max_bw: u32,
               max_power: i8, flags: RegFlags) -> Self {
        Self {
            freq_start,
            freq_end,
            max_bw,
            max_power,
            flags,
            dfs_cac_time: 0,
        }
    }

    pub fn contains_freq(&self, freq_mhz: u32) -> bool {
        self.freq_start <= freq_mhz && freq_mhz <= self.freq_end
    }

    pub fn max_power_dbm(&self) -> i8 {
        self.max_power
    }
}

/// Regulatory domain
#[derive(Clone, Debug)]
pub struct RegDomain {
    pub alpha2: [u8; 2],
    pub name: String,
    pub rules: Vec<RegRule>,
    pub dfs_region: u8,
}

impl RegDomain {
    pub fn new(alpha2: &str) -> Self {
        let mut alpha2_arr = [0u8; 2];
        let bytes = alpha2.as_bytes();
        if bytes.len() >= 2 {
            alpha2_arr[0] = bytes[0];
            alpha2_arr[1] = bytes[1];
        }
        Self {
            alpha2: alpha2_arr,
            name: String::new(),
            rules: Vec::new(),
            dfs_region: 0,
        }
    }

    pub fn alpha2_str(&self) -> &str {
        core::str::from_utf8(&self.alpha2).unwrap_or("XX")
    }

    pub fn add_rule(&mut self, rule: RegRule) {
        self.rules.push(rule);
    }

    pub fn get_rule(&self, freq_mhz: u32) -> Option<&RegRule> {
        self.rules.iter().find(|r| r.contains_freq(freq_mhz))
    }

    pub fn max_power_dbm(&self, freq_mhz: u32) -> Option<i8> {
        self.rules.iter()
            .filter(|r| r.contains_freq(freq_mhz))
            .map(|r| r.max_power)
            .max()
    }

    pub fn max_bw_khz(&self, freq_mhz: u32) -> Option<u32> {
        self.rules.iter()
            .filter(|r| r.contains_freq(freq_mhz))
            .map(|r| r.max_bw)
            .max()
    }

    pub fn is_dfs(&self, freq_mhz: u32) -> bool {
        self.rules.iter()
            .filter(|r| r.contains_freq(freq_mhz))
            .any(|r| r.flags.contains(RegFlags::DFS))
    }
}

/// Regulatory domain database
pub struct RegulatoryDb {
    domains: BTreeMap<String, RegDomain>,
    current: Option<String>,
    global: RegDomain,
}

impl RegulatoryDb {
    pub fn new() -> Self {
        let mut db = Self {
            domains: BTreeMap::new(),
            current: None,
            global: RegDomain::new("00"),
        };

        // Add world regulatory domain (00)
        let mut world = RegDomain::new("00");
        world.name = "World".to_string();
        world.dfs_region = 0;
        world.add_rule(RegRule::new(2402, 2482, 20000, 20, RegFlags::empty()));
        world.add_rule(RegRule::new(5170, 5825, 20000, 20, RegFlags::empty()));
        db.add_domain(world);

        // US (FCC)
        let mut us = RegDomain::new("US");
        us.name = "United States".to_string();
        us.dfs_region = 0;
        us.add_rule(RegRule::new(2402, 2472, 20000, 30, RegFlags::empty()));
        us.add_rule(RegRule::new(5170, 5250, 20000, 30, RegFlags::DFS));
        us.add_rule(RegRule::new(5250, 5350, 20000, 23, RegFlags::DFS));
        us.add_rule(RegRule::new(5470, 5725, 20000, 23, RegFlags::DFS));
        us.add_rule(RegRule::new(5745, 5825, 20000, 30, RegFlags::empty()));
        us.add_rule(RegRule::new(5925, 7125, 20000, 30, RegFlags::empty()));
        db
    }

    fn add_domain(&mut self, domain: RegDomain) {
        self.domains.insert(domain.alpha2_str().to_string(), domain);
    }
}

impl Default for RegulatoryDb {
    fn default() -> Self {
        Self::new()
    }
}

/// Global regulatory database instance
static mut REGDB: Option<RegulatoryDb> = None;

/// Initialize the regulatory database
pub fn init() {
    unsafe {
        REGDB = Some(RegulatoryDb::default());
    }
}

/// Get the global regulatory database
pub fn regdb() -> &'static RegulatoryDb {
    unsafe { REGDB.as_ref().expect("Regulatory DB not initialized") }
}

/// Get the current regulatory domain
pub fn current_domain() -> Option<&'static RegDomain> {
    unsafe { REGDB.as_ref().and_then(|db| db.current.as_ref().and_then(|c| db.domains.get(c))) }
}

/// Set the current regulatory domain
pub fn set_current(alpha2: &str) -> bool {
    unsafe {
        if let Some(ref mut regdb) = REGDB {
            if regdb.domains.contains_key(alpha2) {
                regdb.current = Some(alpha2.to_string());
                return true;
            }
        }
    }
    false
}

/// Check if a frequency is allowed
pub fn freq_allowed(freq_mhz: u32, _bw_khz: u32) -> bool {
    unsafe {
        if let Some(ref regdb) = REGDB {
            if let Some(ref current) = regdb.current {
                if let Some(domain) = regdb.domains.get(current) {
                    return domain.get_rule(freq_mhz).is_some();
                }
            }
            return regdb.global.get_rule(freq_mhz).is_some();
        }
    }
    true
}

/// Get max power for frequency
pub fn max_power_dbm(freq_mhz: u32) -> Option<i8> {
    unsafe {
        REGDB.as_ref().and_then(|db| {
            db.current.as_ref().and_then(|c| db.domains.get(c))
                .or(Some(&db.global))
                .and_then(|d| d.max_power_dbm(freq_mhz))
        })
    }
}

/// Get max bandwidth for frequency
pub fn max_bandwidth_khz(freq_mhz: u32) -> Option<u32> {
    unsafe {
        REGDB.as_ref().and_then(|db| {
            db.current.as_ref().and_then(|c| db.domains.get(c))
                .or(Some(&db.global))
                .and_then(|d| d.max_bw_khz(freq_mhz))
        })
    }
}

/// Check if frequency requires DFS
pub fn requires_dfs(freq_mhz: u32) -> bool {
    unsafe {
        REGDB.as_ref().and_then(|db| {
            db.current.as_ref().and_then(|c| db.domains.get(c))
                .or(Some(&db.global))
                .map(|d| d.is_dfs(freq_mhz))
        }).unwrap_or(false)
    }
}

/// Get DFS CAC time for frequency
pub fn dfs_cac_time(freq_mhz: u32) -> Option<u16> {
    unsafe {
        REGDB.as_ref().and_then(|db| {
            db.current.as_ref().and_then(|c| db.domains.get(c))
                .or(Some(&db.global))
                .and_then(|d| d.get_rule(freq_mhz).map(|r| r.dfs_cac_time))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regdom_creation() {
        let mut reg = RegDomain::new("US");
        assert_eq!(reg.alpha2_str(), "US");
    }
}