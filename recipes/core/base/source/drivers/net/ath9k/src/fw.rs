//! Firmware loading for ath9k.
//!
//! Tries a real blob from the filesystem (initfs), falling back to a
//! deterministic mock so the driver can complete bring-up in VMs and CI.

use alloc::sync::Arc;
use alloc::vec::Vec;

/// ath9k firmware image.
#[derive(Debug, Default)]
pub struct Firmware {
    data: Vec<u8>,
    version: &'static str,
    real: bool,
}

impl Firmware {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load firmware from `path`; falls back to a mock blob when missing.
    pub fn load(path: &str) -> Result<Arc<Self>, &'static str> {
        log::info!("ath9k: loading firmware from {}", path);

        let (data, version, real) = match std::fs::read(path) {
            Ok(bytes) => (bytes, "from-blob", true),
            Err(err) => {
                log::warn!("ath9k: firmware {} unavailable ({}), using mock", path, err);
                (Self::mock_blob(), "mock-1.0", false)
            }
        };

        if data.is_empty() {
            return Err("empty firmware");
        }

        Ok(Arc::new(Self { data, version, real }))
    }

    fn mock_blob() -> Vec<u8> {
        // Deterministic stand-in with a header + payload.
        let mut v = Vec::with_capacity(64);
        v.extend_from_slice(b"ATH9KFW\0");
        v.extend_from_slice(&[0xA5; 56]);
        v
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data, version: "custom", real: true }
    }

    /// Whether the image came from a real on-disk blob.
    pub fn is_real(&self) -> bool {
        self.real
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_by_default() {
        let fw = Firmware::new();
        assert!(fw.is_empty());
        assert_eq!(fw.len(), 0);
        assert!(!fw.is_real());
    }

    #[test]
    fn keeps_bytes() {
        let fw = Firmware::from_bytes(alloc::vec![1, 2, 3, 4]);
        assert_eq!(fw.len(), 4);
        assert!(!fw.is_empty());
        assert_eq!(fw.version(), "custom");
    }

    #[test]
    fn load_missing_path_falls_back_to_mock() {
        let fw = Firmware::load("/nonexistent/ath9k/htc_9271.fw").expect("mock fallback");
        assert!(!fw.is_empty());
        assert!(!fw.is_real());
        assert_eq!(fw.version(), "mock-1.0");
    }

    #[test]
    fn load_real_file() {
        let dir = std::env::temp_dir().join("ath9k_fw_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("htc_9271.fw");
        std::fs::write(&path, [0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

        let fw = Firmware::load(path.to_str().unwrap()).expect("real blob");
        assert_eq!(fw.len(), 4);
        assert!(fw.is_real());
        assert_eq!(fw.version(), "from-blob");

        std::fs::remove_file(&path).ok();
    }
}
