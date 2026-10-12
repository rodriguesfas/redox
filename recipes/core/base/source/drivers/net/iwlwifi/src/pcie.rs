//! PCIe interface for Intel iwlwifi

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

pub struct PcieTransport {
    regs: *mut u32,
    tx_queue: TxQueue,
    rx_queue: RxQueue,
    doorbell: AtomicU32,
}

struct TxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
    // Each entry: [address_low, address_high, length, flags]
}

struct RxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
    // Each entry: [address_low, address_high, length, flags]
}

impl PcieTransport {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("iwlwifi PCIe: Initializing transport");

        // Initialize TX queue (256 entries)
        let tx_queue = TxQueue {
            base: unsafe { regs.add(0x1000) },
            count: 256,
            head: 0,
            tail: 0,
        };

        // Initialize RX queue (256 entries)
        let rx_queue = RxQueue {
            base: unsafe { regs.add(0x2000) },
            count: 256,
            head: 0,
            tail: 0,
        };

        // Enable DMA
        unsafe {
            // FH_RSCSR_CHNL0_TX_RING_BASE
            core::ptr::write_volatile(regs.add(0x0300), 0x1);
            // FH_RSCSR_CHNL0_RX_RING_BASE
            core::ptr::write_volatile(regs.add(0x0400), 0x1);
            // FH_MEM_RCSR_CHNL0_CONFIG_REG
            core::ptr::write_volatile(regs.add(0x0118), 0x3);
        }

        Ok(Self {
            regs,
            tx_queue,
            rx_queue,
            doorbell: AtomicU32::new(0),
        })
    }

    pub fn tx_write(&mut self, data: &[u8]) -> Result<usize, &'static str> {
        let tail = self.tx_queue.tail;
        let next_tail = (tail + 1) % self.tx_queue.count;

        if next_tail == self.tx_queue.head {
            return Err("TX queue full");
        }

        // Write descriptor
        let desc_base = unsafe { self.tx_queue.base.add(tail * 4) };
        unsafe {
            // In real implementation, would set up DMA mapping
            core::ptr::write_volatile(desc_base.add(0), data.as_ptr() as u32);
            core::ptr::write_volatile(desc_base.add(1), 0); // high addr
            core::ptr::write_volatile(desc_base.add(2), data.len() as u32);
            core::ptr::write_volatile(desc_base.add(3), 0x1); // flags: valid
        }

        self.tx_queue.tail = next_tail;

        // Ring doorbell
        self.ring_doorbell(0);

        Ok(data.len())
    }

    pub fn rx_read(&mut self, buf: &mut [u8]) -> Result<usize, &'static str> {
        if self.rx_queue.head == self.rx_queue.tail {
            return Err("RX queue empty");
        }

        let head = self.rx_queue.head;
        let desc_base = unsafe { self.rx_queue.base.add(head * 4) };

        let length = unsafe { core::ptr::read_volatile(desc_base.add(2)) } as usize;
        let data_ptr = unsafe { core::ptr::read_volatile(desc_base.add(0)) } as *const u8;

        let copy_len = core::cmp::min(length, buf.len());
        unsafe {
            core::ptr::copy_nonoverlapping(data_ptr, buf.as_mut_ptr(), copy_len);
        }

        self.rx_queue.head = (head + 1) % self.rx_queue.count;

        Ok(copy_len)
    }

    fn ring_doorbell(&self, queue_id: u32) {
        let val = (queue_id << 16) | 0x1;
        self.doorbell.store(val, Ordering::Release);
        unsafe {
            core::ptr::write_volatile(self.regs.add(0x0120), val);
        }
    }

    pub fn tx_complete(&mut self) -> usize {
        let mut completed = 0;
        while self.tx_queue.head != self.tx_queue.tail {
            let head = self.tx_queue.head;
            let desc_base = unsafe { self.tx_queue.base.add(head * 4) };
            let flags = unsafe { core::ptr::read_volatile(desc_base.add(3)) };

            if flags & 0x80000000 == 0 {
                break; // Not done yet
            }

            self.tx_queue.head = (head + 1) % self.tx_queue.count;
            completed += 1;
        }
        completed
    }

    pub fn rx_available(&self) -> usize {
        if self.rx_queue.tail >= self.rx_queue.head {
            self.rx_queue.tail - self.rx_queue.head
        } else {
            self.rx_queue.count - self.rx_queue.head + self.rx_queue.tail
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcie_transport_creation() {
        // Can't easily test without real hardware registers
        // This is a placeholder for when we have mock registers
    }

    #[test]
    fn test_queue_full_empty() {
        // Test queue full/empty conditions
    }
}