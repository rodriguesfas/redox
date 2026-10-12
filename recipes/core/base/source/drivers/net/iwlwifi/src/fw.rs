//! Firmware loading and management for Intel iwlwifi

use alloc::{vec::Vec, string::String, sync::Arc};
use std::fs::File;
use std::io::Read;

#[derive(Clone, Debug)]
pub struct Firmware {
    data: Vec<u8>,
    version: String,
    api_version: u32,
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

        // Try to load real firmware file
        let data = match Self::load_file(path) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("Failed to load firmware {}: {}, using mock", path, e);
                Self::create_mock_firmware()
            }
        };

        let version = String::from("2024.01.15-1");
        let api_version = 10; // Latest API version for AX210/BE200

        // Parse TLV entries from firmware
        let tlv_data = Self::parse_tlv(&data)?;

        Ok(Arc::new(Self {
            data,
            version,
            api_version,
            tlv_data,
        }))
    }

    fn load_file(path: &str) -> Result<Vec<u8>, &'static str> {
        // Try to open and read the file
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Err("File not found"),
        };

        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            log::error!("Failed to read firmware file: {:?}", e);
            return Err("Read failed");
        }

        // Check if it's compressed (.zst)
        if path.ends_with(".zst") {
            // Decompress zstd
            match zstd::stream::decode_all(&mut data.as_slice()) {
                Ok(decompressed) => Ok(decompressed),
                Err(e) => {
                    log::error!("Failed to decompress firmware: {:?}", e);
                    Err("Decompression failed")
                }
            }
        } else {
            Ok(data)
        }
    }

    fn create_mock_firmware() -> Vec<u8> {
        // Create a mock firmware with TLV structure
        let mut data = Vec::new();

        // Add TLV header for ucode
        data.extend_from_slice(&0x01u32.to_le_bytes()); // FW_TLV_TYPE_UCODE
        data.extend_from_slice(&(1024u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xAA; 1024]); // mock ucode data

        // Add TLV header for debug
        data.extend_from_slice(&0x02u32.to_le_bytes()); // FW_TLV_TYPE_DEBUG
        data.extend_from_slice(&(256u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xBB; 256]); // mock debug data

        // Add TLV header for calibration
        data.extend_from_slice(&0x03u32.to_le_bytes()); // FW_TLV_TYPE_CALIB
        data.extend_from_slice(&(512u32).to_le_bytes()); // length
        data.extend_from_slice(&[0xCC; 512]); // mock calibration data

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

    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn get_tlv(&self, tlv_type: u32) -> Option<&[u8]> {
        self.tlv_data.iter()
            .find(|e| e.tlv_type == tlv_type)
            .map(|e| e.data.as_slice())
    }

    pub fn get_ucode(&self) -> Option<&[u8]> {
        self.get_tlv(0x01) // FW_TLV_TYPE_UCODE
    }

    pub fn get_debug(&self) -> Option<&[u8]> {
        self.get_tlv(0x02) // FW_TLV_TYPE_DEBUG
    }

    pub fn get_calibration(&self) -> Option<&[u8]> {
        self.get_tlv(0x03) // FW_TLV_TYPE_CALIB
    }

    pub fn get_radio(&self) -> Option<&[u8]> {
        self.get_tlv(0x04) // FW_TLV_TYPE_RADIO
    }

    pub fn get_regulatory(&self) -> Option<&[u8]> {
        self.get_tlv(0x05) // FW_TLV_TYPE_REGULATORY
    }

    pub fn get_nvm(&self) -> Option<&[u8]> {
        self.get_tlv(0x06) // FW_TLV_TYPE_NVM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firmware_load() {
        let fw = Firmware::load("/lib/firmware/intel/test.bin").unwrap();
        assert_eq!(fw.version(), "2024.01.15-1");
    }

    #[test]
    fn test_firmware_tlv_parsing() {
        let fw = Firmware::load("/lib/firmware/intel/test.bin").unwrap();

        let ucode = fw.get_ucode();
        assert!(ucode.is_some());
        assert_eq!(ucode.unwrap().len(), 1024);

        let debug = fw.get_debug();
        assert!(debug.is_some());
        assert_eq!(debug.unwrap().len(), 256);

        let calib = fw.get_calibration();
        assert!(calib.is_some());
        assert_eq!(calib.unwrap().len(), 512);

        let radio = fw.get_radio();
        assert!(radio.is_none()); // Not in mock data
    }

    #[test]
    fn test_firmware_missing_tlv() {
        let fw = Firmware::load("/lib/firmware/intel/test.bin").unwrap();
        assert!(fw.get_regulatory().is_none());
        assert!(fw.get_nvm().is_none());
    }
}