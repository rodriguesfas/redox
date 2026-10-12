//! Debug utilities for MT7921

pub fn dump_registers(regs: *mut u32) {
    log::info!("=== MT7921 Register Dump ===");
    
    // WPDMA
    log::info!("WPDMA_GLO_CFG: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x0000)) });
    
    // Interrupts
    log::info!("INT_ENABLE: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x0200)) });
    log::info!("INT_STATUS: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x0204)) });
    log::info!("INT_ACK: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x0208)) });
    
    // MAC
    log::info!("MAC_TX_CTRL: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x1100)) });
    log::info!("MAC_RX_CTRL: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x1200)) });
    
    // MCU
    log::info!("MCU_CTRL: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x5000)) });
    log::info!("MCU_INT_ENABLE: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x5100)) });
    log::info!("MCU_INT_STATUS: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x5104)) });
    
    // PHY
    log::info!("PHY_CTRL: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x4000)) });
    log::info!("PHY_CHANNEL: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x4200)) });
    log::info!("PHY_BAND: {:#x}", unsafe { core::ptr::read_volatile(regs.add(0x4204)) });
}

pub fn print_stats(irq_count: u64, tx_packets: u64, rx_packets: u64) {
    log::info!("MT7921 Stats: IRQs={}, TX={}, RX={}", irq_count, tx_packets, rx_packets);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_stats_format() {
        // Just verify the function signature exists
        // Actual logging would require a logger setup
        print_stats(100, 1000, 500);
    }
}