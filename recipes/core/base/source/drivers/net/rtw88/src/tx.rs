//! TX (Transmission) path for Realtek rtw88

use alloc::vec::Vec;
use alloc::boxed::Box;
use crate::regs::*;

pub struct TxEngine {
    regs: *mut u32,
    queues: [TxQueue; 8],
    current_queue: usize,
}

struct TxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
    queue_id: u8,
}

impl TxEngine {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("rtw88 TX: Initializing transmission engine");

        let mut queues: [TxQueue; 8] = core::array::from_fn(|i| TxQueue {
            base: unsafe { regs.add(REG_TXQ_BASE + i * 0x100) },
            count: 256,
            head: 0,
            tail: 0,
            queue_id: i as u8,
        });

        // Enable TX DMA
        unsafe {
            core::ptr::write_volatile(regs.add(REG_TXQ_CTRL), TXQ_CTRL_EN);
        }

        Ok(Self {
            regs,
            queues,
            current_queue: 0,
        })
    }

    pub fn transmit(&mut self, queue_id: u8, data: &[u8], flags: TxFlags) -> Result<u32, &'static str> {
        let queue_idx = queue_id as usize;
        if queue_idx >= self.queues.len() {
            return Err("Invalid queue ID");
        }

        let queue = &mut self.queues[queue_idx];
        let next_tail = (queue.tail + 1) % queue.count;

        if next_tail == queue.head {
            return Err("TX queue full");
        }

        // Build TX descriptor
        let desc = TxDescriptor {
            addr_low: data.as_ptr() as u32,
            addr_high: 0,
            length: data.len() as u32,
            flags: flags.bits(),
            queue_id,
            reserved: 0,
        };

        // Write descriptor to ring
        let desc_offset = queue.tail * core::mem::size_of::<TxDescriptor>();
        unsafe {
            let desc_ptr = queue.base.add(desc_offset) as *mut TxDescriptor;
            core::ptr::write_volatile(desc_ptr, desc);
        }

        queue.tail = next_tail;

        // Ring doorbell for this queue
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_TXQ_CTRL + queue_idx * 4), TXQ_CTRL_EN);
        }

        Ok(queue.tail as u32)
    }

    pub fn transmit_aggregated(&mut self, queue_id: u8, frames: &[&[u8]], ba_session: &BaSession) -> Result<usize, &'static str> {
        let queue_idx = queue_id as usize;
        if queue_idx >= self.queues.len() {
            return Err("Invalid queue ID");
        }

        let queue = &mut self.queues[queue_idx];
        let mut transmitted = 0;

        for frame in frames {
            let next_tail = (queue.tail + 1) % queue.count;
            if next_tail == queue.head {
                break; // Queue full
            }

            let desc = TxDescriptor {
                addr_low: frame.as_ptr() as u32,
                addr_high: 0,
                length: frame.len() as u32,
                flags: TxFlags::AGG.bits() | TxFlags::BA.bits(),
                queue_id,
                reserved: ba_session.sta_id as u32 | ((ba_session.tid as u32) << 8),
            };

            let desc_offset = queue.tail * core::mem::size_of::<TxDescriptor>();
            unsafe {
                let desc_ptr = queue.base.add(desc_offset) as *mut TxDescriptor;
                core::ptr::write_volatile(desc_ptr, desc);
            }

            queue.tail = next_tail;
            transmitted += 1;
        }

        if transmitted > 0 {
            unsafe {
                core::ptr::write_volatile(self.regs.add(REG_TXQ_CTRL + queue_idx * 4), TXQ_CTRL_EN);
            }
        }

        Ok(transmitted)
    }

    pub fn handle_tx_done(&mut self, queue_id: u8) -> usize {
        let queue_idx = queue_id as usize;
        if queue_idx >= self.queues.len() {
            return 0;
        }

        let queue = &mut self.queues[queue_idx];
        let mut completed = 0;

        while queue.head != queue.tail {
            let desc_offset = queue.head * core::mem::size_of::<TxDescriptor>();
            let desc_ptr = unsafe { queue.base.add(desc_offset) as *const TxDescriptor };
            let desc = unsafe { core::ptr::read_volatile(desc_ptr) };

            if desc.flags & TX_DESC_OWN != 0 {
                break; // Still owned by hardware
            }

            queue.head = (queue.head + 1) % queue.count;
            completed += 1;
        }

        completed
    }

    pub fn get_queue_depth(&self, queue_id: u8) -> usize {
        let queue_idx = queue_id as usize;
        if queue_idx >= self.queues.len() {
            return 0;
        }

        let queue = &self.queues[queue_idx];
        if queue.tail >= queue.head {
            queue.tail - queue.head
        } else {
            queue.count - queue.head + queue.tail
        }
    }

    pub fn flush_queue(&mut self, queue_id: u8) -> Result<(), &'static str> {
        let queue_idx = queue_id as usize;
        if queue_idx >= self.queues.len() {
            return Err("Invalid queue ID");
        }

        // Wait for all packets to be transmitted
        while self.get_queue_depth(queue_id) > 0 {
            core::hint::spin_loop();
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct TxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
    queue_id: u8,
    reserved: u32,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct TxFlags: u32 {
        const OWN = 0x80000000;
        const FS = 0x40000000;
        const LS = 0x20000000;
        const AGG = 0x10000000;      // Aggregated frame
        const BA = 0x08000000;       // Block Ack
        const RTS = 0x04000000;      // RTS/CTS
        const CTS = 0x02000000;      // CTS-to-self
        const HW_SEQ = 0x01000000;   // Hardware sequence
        const MORE = 0x00800000;     // More fragments
        const ENCRYPT = 0x00400000;  // Encrypt
        const VALID = 0x00000001;    // Valid
    }
}

#[derive(Clone, Debug)]
pub struct BaSession {
    pub sta_id: u8,
    pub tid: u8,
    pub ssn: u16,
    pub buf_size: u16,
    pub timeout: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_flags() {
        assert_eq!(TxFlags::OWN.bits(), 0x80000000);
        assert_eq!(TxFlags::FS.bits(), 0x40000000);
        assert_eq!(TxFlags::LS.bits(), 0x20000000);
        assert_eq!(TxFlags::AGG.bits(), 0x10000000);
        assert_eq!(TxFlags::BA.bits(), 0x08000000);

        let combined = TxFlags::OWN | TxFlags::FS | TxFlags::LS | TxFlags::AGG;
        assert!(combined.contains(TxFlags::OWN));
        assert!(combined.contains(TxFlags::FS));
        assert!(combined.contains(TxFlags::LS));
    }

    #[test]
    fn test_tx_descriptor_size() {
        assert_eq!(core::mem::size_of::<TxDescriptor>(), 24);
    }

    #[test]
    fn test_ba_session() {
        let session = BaSession {
            sta_id: 1,
            tid: 0,
            ssn: 100,
            buf_size: 64,
            timeout: 0,
        };
        assert_eq!(session.sta_id, 1);
        assert_eq!(session.tid, 0);
        assert_eq!(session.ssn, 100);
    }
}