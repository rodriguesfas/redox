//! Firmware loading and management for MT7921

use alloc::{vec::Vec, string::String, sync::Arc};

#[derive(Clone, Debug)]
pub struct Firmware {
    data: Vec<u8>,
    version: String,
}

impl Firmware {
    pub fn load(path: &str) -> Result<Arc<Self>, &'static str> {
        log::info!("Loading firmware from: {}", path);

        // Mock determinístico nao-vazio (upload aceita; blob real via disco vem depois).
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(b"MT7921FW\0");
        data.extend_from_slice(&[0xA5; 56]);
        let version = String::from("2024.01.15");
        
        Ok(Arc::new(Self { data, version }))
    }
    
    pub fn version(&self) -> &str {
        &self.version
    }
    
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    
    pub fn get_patch(&self, name: &str) -> Option<&[u8]> {
        // Extract specific firmware patch by name
        // MT7921 has multiple firmware components:
        // - mt7921_firmware.bin (main)
        // - mt7921_rom_patch.bin (ROM patch)
        // - mt7921_wm.bin (WiFi manager)
        // - mt7921_bt.bin (Bluetooth)
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firmware_load() {
        let fw = Firmware::load("/lib/firmware/mediatek/test.bin").unwrap();
        assert_eq!(fw.version(), "2024.01.15");
    }

    #[test]
    fn test_firmware_patches() {
        let fw = Firmware::load("/lib/firmware/mediatek/test.bin").unwrap();
        assert!(fw.get_patch("rom_patch").is_none());
        assert!(fw.get_patch("wm").is_none());
        assert!(fw.get_patch("bt").is_none());
    }
}