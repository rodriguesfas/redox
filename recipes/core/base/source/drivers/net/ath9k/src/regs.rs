//! ath9k register offsets (simplified map, in 32-bit words).

/// MAC register offsets
pub const REG_TX_CTRL: usize = 0x0100;
pub const REG_RX_CTRL: usize = 0x0104;
pub const REG_MAC_ADDRESS: usize = 0x0500;

/// Interrupt registers
pub const REG_INT_STATUS: usize = 0x0300;
pub const REG_INT_CLEAR: usize = 0x0304;

/// Interrupt bits
pub const INT_TX_DONE: u32 = 0x0000_0001;
pub const INT_RX_DONE: u32 = 0x0000_0002;
pub const INT_TBTT: u32 = 0x0000_0004;

/// PHY registers
pub const REG_PHY_CTRL: usize = 0x0400;
pub const REG_PHY_BANDWIDTH: usize = 0x0418;

/// Power management
pub const REG_PM_CTRL: usize = 0x0700;

/// Reset / firmware upload (staging SRAM window + control)
pub const REG_RESET: usize = 0x0004;
pub const REG_RESET_BIT: u32 = 0x0000_0001;
/// Base offset (in u32 words) of the SRAM staging window for firmware blobs.
pub const REG_FW_SRAM_WINDOW: usize = 0x2000;
pub const REG_FW_CTRL: usize = 0x2010;
pub const REG_FW_STATUS: usize = 0x2011;
/// FW_CTRL: write 1 to start execution of the uploaded image.
pub const FW_CTRL_START: u32 = 0x0000_0001;
/// FW_STATUS: read 0 = engine ready/idle (accepts upload or running).
pub const FW_STATUS_BUSY: u32 = 0x0000_0001;

// ==== RX/TX descriptor rings ====
/// Descriptors per ring.
pub const RING_LEN: u32 = 32;
/// Descriptor: {off_words, len_bytes, flags, rsvd} — 4 words.
pub const RING_DESC_WORDS: usize = 4;
/// Word offset of the TX/RX descriptor rings.
pub const REG_TXQ_RING: usize = 0x2800;
pub const REG_RXQ_RING: usize = 0x2880;
/// Ring head/tail registers (monotonic counters; slot = counter % RING_LEN).
pub const REG_TXQ_HEAD: usize = 0x2A00;
pub const REG_TXQ_TAIL: usize = 0x2A01;
pub const REG_RXQ_HEAD: usize = 0x2A02;
pub const REG_RXQ_TAIL: usize = 0x2A03;
/// Word offsets where frame payloads live (fixed 2048-byte slots).
pub const TX_DATA_WINDOW: usize = 0x4000;
pub const RX_DATA_WINDOW: usize = 0x8000;
pub const FRAME_SLOT_BYTES: usize = 2048;
pub const FRAME_SLOT_WORDS: usize = FRAME_SLOT_BYTES / 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_offsets_distinct() {
        let offs = [
            REG_TX_CTRL,
            REG_RX_CTRL,
            REG_INT_STATUS,
            REG_INT_CLEAR,
            REG_PHY_CTRL,
            REG_PHY_BANDWIDTH,
            REG_PM_CTRL,
            REG_MAC_ADDRESS,
        ];
        for i in 0..offs.len() {
            for j in (i + 1)..offs.len() {
                assert_ne!(offs[i], offs[j]);
            }
        }
    }

    #[test]
    fn irq_bits_distinct() {
        assert_eq!(INT_TX_DONE & INT_RX_DONE, 0);
        assert_eq!(INT_TX_DONE & INT_TBTT, 0);
        assert_eq!(INT_RX_DONE & INT_TBTT, 0);
    }

    #[test]
    fn ring_layout_sane() {
        use super::*;
        assert_eq!(REG_TXQ_RING + RING_LEN as usize * RING_DESC_WORDS, REG_RXQ_RING);
        assert!(REG_RXQ_RING + RING_LEN as usize * RING_DESC_WORDS <= REG_TXQ_HEAD);
        // fw staging (0x2000..) e rings/data nao podem se sobrepor
        assert!(REG_FW_CTRL < REG_TXQ_RING);
        // TX window (32 x 512 words) nao pode invadir a RX window
        assert!(TX_DATA_WINDOW + RING_LEN as usize * FRAME_SLOT_WORDS <= RX_DATA_WINDOW);
    }

    #[test]
    fn fw_regs_layout() {
        assert_eq!(REG_FW_CTRL, 0x2010);
        assert_eq!(REG_FW_STATUS, 0x2011);
        assert!(REG_FW_SRAM_WINDOW < REG_FW_CTRL);
    }
}
