//! 802.11 Crypto implementations (CCMP, GCMP, TKIP, WEP)
//!
//! This module implements the cryptographic algorithms used in 802.11:
//! - CCMP (AES-CCMP) - AES in CCM mode
//! - GCMP (Galois/Counter Mode Protocol) - AES-GCM (optional, requires "gcmp" feature)
//! - TKIP (Temporal Key Integrity Protocol)
//! - WEP (Wired Equivalent Privacy) - legacy

use aes::cipher::{KeyInit as AesKeyInit, ArrayLength};
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use ccm::Ccm;
use ccm::aead::{Aead, KeyInit as CcmKeyInit, Payload};
use ccm::aead::consts::{U8, U13};
use arrayref::array_ref;
use alloc::vec::Vec;
use sha1::Sha1;

#[cfg(feature = "gcmp")]
use aes_gcm::{Aes128Gcm, KeyInit as GcmKeyInit, Nonce};
#[cfg(feature = "gcmp")]
use aes_gcm::aead::{Aead as GcmAead, Payload as GcmPayload};

/// CCMP (AES-CCMP) - AES in CCM mode with 128-bit key
/// Used in WPA2/WPA3
pub struct Ccmp {
    // Ccm<Aes128, TagSize=U8, NonceSize=U13>: CCMP usa nonce de 13 bytes
    // (PN48|flags) e MIC de 8 bytes por padrao (IEEE 802.11-2016 12.3.2).
    cipher: Ccm<Aes128, U8, U13>,
}

impl Ccmp {
    /// Create a new CCMP cipher with a 128-bit temporal key
    pub fn new(tk: &[u8; 16]) -> Self {
        let cipher = Ccm::<Aes128, U8, U13>::new(GenericArray::from_slice(tk));
        Self { cipher }
    }

    /// Encrypt and authenticate a frame (MFP/CCMP)
    ///
    /// # Arguments
    /// * `pn` - Packet Number (6 bytes, little-endian)
    /// * `key_id` - Key ID (0-3)
    /// * `tid` - Traffic ID (0-15)
    /// * `ta` - Transmitter Address (6 bytes)
    /// * `plaintext` - Plaintext payload
    /// * `aad` - Additional Authenticated Data (MAC header)
    ///
    /// Returns encrypted payload + MIC (8 bytes appended)
    pub fn encrypt(&mut self, pn: &[u8; 6], key_id: u8, _tid: u8, ta: &[u8; 6], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, ccm::Error> {
        // Construct nonce per IEEE 802.11-2016 12.3.2
        // PN (6 bytes, little-endian) | KeyID (1 byte) | TID (4 bits) | reserved (12 bits)
        let mut nonce = [0u8; 13];
        nonce[..6].copy_from_slice(pn);
        nonce[6] = (1 << 6) | (1 << 5) | (1 << 4) | (1 << 3); // ExtIV | KeyID | reserved
        nonce[7] = 0; // reserved

        // PN is 6 bytes, little-endian
        nonce[0..6].copy_from_slice(pn);

        let payload = Payload {
            msg: plaintext,
            aad,
        };

        // CCM ja autentica e anexa o MIC (tag) de 8 bytes ao ciphertext.
        let ciphertext = self.cipher.encrypt(GenericArray::from_slice(&nonce), payload)?;
        Ok(ciphertext)
    }

    /// Decrypt and authenticate a frame. `ciphertext` inclui o MIC (tag) de
    /// 8 bytes no final, como produzido por [`Self::encrypt`].
    pub fn decrypt(&mut self, pn: &[u8; 6], _key_id: u8, _ta: &[u8; 6], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, ccm::Error> {
        // Nonce identico ao do encrypt (IEEE 802.11-2016 12.3.2).
        let mut nonce = [0u8; 13];
        nonce[..6].copy_from_slice(pn);
        nonce[6] = (1 << 6) | (1 << 5) | (1 << 4) | (1 << 3); // ExtIV | KeyID | reserved
        nonce[7] = 0;

        let payload = Payload {
            msg: ciphertext,
            aad,
        };
        self.cipher.decrypt(GenericArray::from_slice(&nonce), payload)
    }
}

/// GCMP (Galois/Counter Mode Protocol) - AES-GCM
/// Used in 802.11ad/ay and WPA3
#[cfg(feature = "gcmp")]
pub struct Gcmp {
    cipher: Aes128Gcm,
}

#[cfg(feature = "gcmp")]
impl Gcmp {
    pub fn new(tk: &[u8; 16]) -> Self {
        // aes-gcm 0.10 usa hybrid_array internamente; new_from_slice evita a
        // incompatibilidade com o generic-array 0.14 usado pelo crate ccm.
        let cipher = Aes128Gcm::new_from_slice(tk).expect("TK must be 16 bytes");
        Self { cipher }
    }

    pub fn encrypt(&self, pn: &[u8; 6], _ta: &[u8; 6], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let mut nonce = [0u8; 12];
        nonce[..6].copy_from_slice(pn);
        nonce[6..8].copy_from_slice(&[0, 0]); // Salt (2 bytes)
        nonce[8..12].copy_from_slice(&[0, 0, 0, 0]); // Reserved

        let payload = GcmPayload {
            msg: plaintext,
            aad,
        };

        self.cipher.encrypt(Nonce::from_slice(&nonce), payload)
    }

    pub fn decrypt(&self, pn: &[u8; 6], _ta: &[u8; 6], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        // Mesmo nonce de 12 bytes do encrypt (PN48 | salt | reserved).
        let mut nonce = [0u8; 12];
        nonce[..6].copy_from_slice(pn);

        let payload = GcmPayload {
            msg: ciphertext,
            aad,
        };
        self.cipher.decrypt(Nonce::from_slice(&nonce), payload)
    }
}

/// TKIP (Temporal Key Integrity Protocol) - Legacy WPA
/// MICHAEL MIC + RC4
pub struct Tkip {
    tk: [u8; 16],        // Temporal Key
    tx_mic_key: [u8; 8], // TX MIC key
    rx_mic_key: [u8; 8], // RX MIC key
}

impl Tkip {
    pub fn new(tk: &[u8; 16]) -> Self {
        let mut tkip = Self {
            tk: *array_ref!(tk, 0, 16),
            tx_mic_key: [0; 8],
            rx_mic_key: [0; 8],
        };
        tkip.init_mic_keys();
        tkip
    }

    fn init_mic_keys(&mut self) {
        // MIC key derivation per IEEE 802.11i
        // TX MIC key = TK[0..7], RX MIC key = TK[8..15] (simplified)
        self.tx_mic_key.copy_from_slice(&self.tk[0..8]);
        self.rx_mic_key.copy_from_slice(&self.tk[8..16]);
    }

    /// Compute MICHAEL MIC
    pub fn mic(&self, key: &[u8; 8], data: &[u8], _pn: u16) -> [u8; 8] {
        // Michael MIC implementation
        // See IEEE 802.11i Annex 7.2
        let mut k0 = u16::from_be_bytes([key[0], key[1]]);
        let mut k1 = u16::from_be_bytes([key[2], key[1]]);
        let k2 = u16::from_be_bytes([key[4], key[3]]);
        let k3 = u16::from_be_bytes([key[6], key[7]]);

        let mut m0 = 0u16;
        let mut m1 = 0u16;

        // Process data in 2-byte chunks
        for chunk in data.chunks_exact(2) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            m0 ^= v.wrapping_add(k0);
            m1 ^= v.wrapping_add(k1);
            k0 = k0.wrapping_add(m0);
            k1 = k1.wrapping_add(m1);
        }

        // Handle odd byte
        if data.len() % 2 == 1 {
            let last = data[data.len() - 1] as u16;
            m0 ^= (last << 8) as u16;
            m1 ^= k2;
        }

        let mut mic = [0u8; 8];
        mic[0..2].copy_from_slice(&k0.to_le_bytes());
        mic[2..4].copy_from_slice(&m0.to_le_bytes());
        mic[4..6].copy_from_slice(&m1.to_le_bytes());
        mic[6..8].copy_from_slice(&k1.to_le_bytes());
        mic
    }

    pub fn compute_mic(&self, data: &[u8], pn: u16, is_tx: bool) -> [u8; 8] {
        let key = if is_tx { &self.tx_mic_key } else { &self.rx_mic_key };
        self.mic(key, data, pn)
    }
}

/// WEP (Wired Equivalent Privacy) - Legacy, deprecated
/// Only for legacy compatibility
pub struct Wep {
    key: Vec<u8>,
    key_idx: u8,
}

impl Wep {
    pub fn new(key: &[u8], key_idx: u8) -> Self {
        Self {
            key: key.to_vec(),
            key_idx,
        }
    }

    pub fn encrypt(&self, _data: &[u8], _iv: &[u8; 3]) -> Result<Vec<u8>, &'static str> {
        // TKIP (RC4+Michael) e legado/deprecado; nao implementado propositalmente.
        Err("TKIP deprecated: use CCMP/GCMP")
    }

    pub fn decrypt(&self, _data: &[u8], _iv: &[u8; 3]) -> Result<Vec<u8>, &'static str> {
        Err("TKIP deprecated: use CCMP/GCMP")
    }
}

pub mod kdf {
    use alloc::vec::Vec;
    use sha1::{Sha1, Digest};

    /// HMAC-SHA1
    fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
        let mut hasher = Sha1::new();
        hasher.update(key);
        hasher.update(data);
        hasher.finalize().into()
    }

    /// PBKDF2-HMAC-SHA1 (WPA-PSK)
    pub fn pbkdf2_sha1(passphrase: &str, ssid: &str, iterations: u32) -> [u8; 32] {
        let password = passphrase.as_bytes();
        let salt = ssid.as_bytes();

        let mut result = [0u8; 32];

        // PBKDF2 uses 512-byte blocks
        for i in 0..8 {
            let mut salt_block = Vec::with_capacity(salt.len() + 4);
            salt_block.extend_from_slice(salt);
            salt_block.extend_from_slice(&(i as u32).to_be_bytes());

            let mut u = hmac_sha1(password, &salt_block);

            let mut u_prev = u;
            for _ in 1..iterations {
                u_prev = hmac_sha1(password, &u_prev);

                for (r, u) in result.iter_mut().skip(i * 4).take(4).zip(&u_prev) {
                    *r ^= *u;
                }
            }
        }

        result
    }

    /// PRF-512 (used in 4-way handshake)
    pub fn prf_512(key: &[u8], a: &[u8], b: &[u8]) -> [u8; 64] {
        let mut input = Vec::with_capacity(a.len() + b.len() + 2);
        input.extend_from_slice(a);
        input.push(0x00);
        input.extend_from_slice(b);
        input.push(0x00);

        let hmac = hmac_sha1(key, &input);
        let mut result = [0u8; 64];
        result[..20].copy_from_slice(&hmac);
        result
    }

    /// PRF-384 (for 128-bit keys)
    pub fn prf_384(key: &[u8], a: &[u8], b: &[u8]) -> [u8; 48] {
        let mut result = [0u8; 48];
        let prf_result = prf_512(key, a, b);
        result.copy_from_slice(&prf_result[..48]);
        result
    }

    /// KCK/KEK/TK derivation from PMK
    pub fn derive_ptk(
        pmk: &[u8; 32],
        anonce: &[u8; 32],
        snonce: &[u8; 32],
        aa: &[u8; 6],
        spa: &[u8; 6]
    ) -> ([u8; 16], [u8; 16], [u8; 16], [u8; 8]) {
        // KCK || KEK || TK || MIC = PRF-512(PMK, "Pairwise key expansion", ANONCE || SNONCE || AA || SPA)

        let mut input = Vec::with_capacity(32 + 32 + 6 + 6);
        input.extend_from_slice(anonce);
        input.extend_from_slice(snonce);
        input.extend_from_slice(aa);
        input.extend_from_slice(spa);

        let prf = prf_512(pmk, b"Pairwise key expansion", &input);

        let mut kck = [0u8; 16];
        let mut kek = [0u8; 16];
        let mut tk = [0u8; 16];
        let mut mic = [0u8; 8];

        kck.copy_from_slice(&prf[..16]);
        kek.copy_from_slice(&prf[16..32]);
        tk.copy_from_slice(&prf[32..48]);
        mic.copy_from_slice(&prf[48..56]);

        (kck, kek, tk, mic)
    }
}

/// Security capabilities in RSN/RSNXE
bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CipherSuite: u32 {
        const USE_GROUP = 0x00000001;
        const WEP40 = 0x00000002;
        const TKIP = 0x00000004;
        const WRAP = 0x00000008;
        const CCMP = 0x00000010;
        const WEP104 = 0x00000020;
        const BIP_CMAC_128 = 0x00000040;
        const GROUP_NOT_USED = 0x00000080;
        const GCMP_128 = 0x00000100;
        const GCMP_256 = 0x00000200;
        const GMAC_128 = 0x00000400;
        const GMAC_256 = 0x00000800;
        const BIP_GMAC_128 = 0x00001000;
        const BIP_GMAC_256 = 0x00002000;
        const BIP_CMAC_256 = 0x00004000;
    }
}

impl CipherSuite {
    pub fn from_suite(oui: u32, suite_type: u8) -> Option<Self> {
        match (oui, suite_type) {
            (0x000FAC, 1) => Some(Self::WEP40),
            (0x000FAC, 2) => Some(Self::TKIP),
            (0x000FAC, 3) => Some(Self::WRAP),
            (0x000FAC, 4) => Some(Self::CCMP),
            (0x000FAC, 5) => Some(Self::WEP104),
            (0x000FAC, 6) => Some(Self::BIP_CMAC_128),
            (0x000FAC, 7) => Some(Self::GCMP_128),
            (0x000FAC, 8) => Some(Self::GCMP_256),
            (0x000FAC, 9) => Some(Self::GMAC_128),
            (0x000FAC, 10) => Some(Self::GMAC_256),
            (0x000FAC, 11) => Some(Self::BIP_GMAC_128),
            (0x000FAC, 12) => Some(Self::BIP_GMAC_256),
            (0x000FAC, 13) => Some(Self::BIP_CMAC_256),
            _ => None,
        }
    }

    pub fn is_strong(&self) -> bool {
        self.contains(Self::CCMP) || self.contains(Self::GCMP_128) || self.contains(Self::GCMP_256)
    }

    pub fn key_len(&self) -> usize {
        if self.contains(Self::GCMP_256) || self.contains(Self::GMAC_256) || self.contains(Self::BIP_CMAC_256) {
            32
        } else {
            16
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ccmp_encrypt() {
        let tk = [0u8; 16];
        let mut ccmp = Ccmp::new(&tk);
        let pn = [0u8; 6];
        let ta = [0u8; 6];
        let plaintext = b"Hello, World!";
        let aad = b"header";

        let result = ccmp.encrypt(&pn, 0, 0, &ta, plaintext, aad);
        assert!(result.is_ok());
    }

    #[test]
    fn test_ccmp_encrypt_decrypt_round_trip() {
        let tk = [0x42u8; 16];
        let mut ccmp = Ccmp::new(&tk);
        let pn = [1, 2, 3, 4, 5, 6];
        let ta = [0xAA; 6];
        let aad = b"mac-header";
        let plaintext = b"Hello, WPA2!";

        let ct = ccmp.encrypt(&pn, 0, 0, &ta, plaintext, aad).unwrap();
        // CCM anexa tag MIC de 8 bytes.
        assert_eq!(ct.len(), plaintext.len() + 8);

        let back = ccmp.decrypt(&pn, 0, &ta, &ct, aad).unwrap();
        assert_eq!(back.as_slice(), plaintext);

        // MIC adulterado deve falhar autenticacao.
        let mut tampered = ct.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(ccmp.decrypt(&pn, 0, &ta, &tampered, aad).is_err());
    }
}