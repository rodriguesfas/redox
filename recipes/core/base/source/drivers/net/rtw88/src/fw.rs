//! Firmware loading and management for Realtek rtw88

use alloc::{vec::Vec, string::String, sync::Arc};

#[derive(Clone, Debug)]
pub struct Firmware {
    data: Vec<u8>,
    version: String,
    tlv_data: Vec<TlvEntry>,
}

#[derive(Debug, Clone)]
struct TlvEntry {
    tlv_type: u32,
    length: u32,
    data: Vec<u8>,
}

impl Firmware {
    pub fn load(path: &str) -> Result<Arc<Self>, &'static str> {
        log::info!("Loading firmware from: {}", path);

        // In a real implementation, this would read from the filesystem
        // For now, we simulate loading with proper structure
        let data = Self::create_mock_firmware();
        let version = String::from("2024.01.15-1");

        // Parse TLV entries from firmware
        let tlv_data = Self::parse_tlv(&data)?;

        Ok(Arc::new(Self {
            data,
            version,
            tlv_data,
        }))
    }

    fn create_mock_firmware() -> Vec<u8> {
        // Create a mock firmware with TLV structure
        let mut data = Vec::new();

        // Add TLV header for main firmware
        data.extend_from_slice(&0x01u32.to_le_bytes()); // FW_TLV_TYPE_MAIN
        data.extend_from_slice(&(2048u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xAA; 2048]); // mock firmware data

        // Add TLV header for BT
        data.extend_from_slice(&0x03u32.to_le_bytes()); // FW_TLV_TYPE_BT
        data.extend_from_slice(&(512u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xBB; 512]); // mock BT data

        // Add TLV header for coex
        data.extend_from_slice(&0x04u32.to_le_bytes()); // FW_TLV_TYPE_COEX
        data.extend_from_slice(&(512u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xCC; 512]); // mock coex data

        data
    }

    fn parse_tlv(data: &[u8]) -> Result<Vec<TlvEntry>, &'static str> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset + 8 <= data.len() {
            let tlv_type = u32::from_le_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3]
            ]);
            let length = u32::from_le_bytes([
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]
            ]);

            if offset + 8 + length as usize > data.len() {
                return Err("Invalid TLV length");
            }

            let tlv_data = data[offset + 8..offset + 8 + length as usize].to_vec();
            entries.push(TlvEntry {
                tlv_type,
                length,
                data: tlv_data,
            });

            offset += 8 + length as usize;
        }

        Ok(entries)
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn get_tlv(&self, tlv_type: u32) -> Option<&[u8]> {
        self.tlv_data.iter()
            .find(|e| e.tlv_type == tlv_type)
            .map(|e| e.data.as_slice())
    }

    pub fn get_main(&self) -> Option<&[u8]> {
        self.get_tlv(0x01) // FW_TLV_TYPE_MAIN
    }

    pub fn get_wow(&self) -> Option<&[u8]> {
        self.get_tlv(0x02) // FW_TLV_TYPE_WOW
    }

    pub fn get_bt(&self) -> Option<&[u8]> {
        self.get_tlv(0x03) // FW_TLV_TYPE_BT
    }

    pub fn get_coex(&self) -> Option<&[u8]> {
        self.get_tlv(0x04) // FW_TLV_TYPE_COEX
    }

    pub fn get_phy_reg(&self) -> Option<&[u8]> {
        self.get_tlv(0x05) // FW_TLV_TYPE_PHY_REG
    }

    pub fn get_rf_reg(&self) -> Option<&[u8]> {
        self.get_tlv(0x06) // FW_TLV_TYPE_RF_REG
    }

    pub fn get_mac_reg(&self) -> Option<&[u8]> {
        self.get_tlv(0x07) // FW_TLV_TYPE_MAC_REG
    }

    pub fn get_txpwr(&self) -> Option<&[u8]> {
        self.get_tlv(0x08) // FW_TLV_TYPE_TXPWR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firmware_load() {
        let fw = Firmware::load("/lib/firmware/realtek/test.bin").unwrap();
        assert_eq!(fw.version(), "2024.01.15-1");
    }

    #[test]
    fn test_firmware_tlv_parsing() {
        let fw = Firmware::load("/lib/firmware/realtek/test.bin").unwrap();

        let main = fw.get_main();
        assert!(main.is_some());
        assert_eq!(main.unwrap().len(), 2048);

        let bt = fw.get_bt();
        assert!(bt.is_some());
        assert_eq!(bt.unwrap().len(), 512);

        let coex = fw.get_coex();
        assert!(coex.is_some());
        assert_eq!(coex.unwrap().len(), 512);

        let wow = fw.get_wow();
        assert!(wow.is_none()); // Not in mock data
    }

    #[test]
    fn test_firmware_missing_tlv() {
        let fw = Firmware::load("/lib/firmware/realtek/test.bin").unwrap();
        assert!(fw.get_wow().is_none());
        assert!(fw.get_phy_reg().is_none());
    }
}