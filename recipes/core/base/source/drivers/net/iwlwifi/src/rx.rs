//! RX (Reception) path for Intel iwlwifi

use alloc::vec::Vec;
use alloc::boxed::Box;
use crate::regs::*;

pub struct RxEngine {
    regs: *mut u32,
    main_queue: RxQueue,
    monitor_queue: RxQueue,
    mcast_queue: RxQueue,
}

struct RxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
    queue_type: RxQueueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RxQueueType {
    Main = 0,
    Monitor = 1,
    Multicast = 2,
}

impl RxEngine {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("iwlwifi RX: Initializing reception engine");

        let main_queue = RxQueue {
            base: unsafe { regs.add(RX_QUEUE_MAIN_BASE) },
            count: 512,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Main,
        };

        let monitor_queue = RxQueue {
            base: unsafe { regs.add(RX_QUEUE_MONITOR_BASE) },
            count: 128,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Monitor,
        };

        let mcast_queue = RxQueue {
            base: unsafe { regs.add(RX_QUEUE_MCAST_BASE) },
            count: 128,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Multicast,
        };

        // Enable RX DMA
        unsafe {
            core::ptr::write_volatile(regs.add(RX_DMA_CTRL), RX_DMA_CTRL_ENABLE);
            // Configure RSSI threshold
            core::ptr::write_volatile(regs.add(RX_RSSI_THRESHOLD), 0xFF);
        }

        Ok(Self {
            regs,
            main_queue,
            monitor_queue,
            mcast_queue,
        })
    }

    pub fn receive(&mut self, buf: &mut [u8]) -> Result<RxResult, &'static str> {
        // Try main queue first
        if let Ok(result) = Self::receive_from_queue_static(&mut self.main_queue, buf) {
            return Ok(result);
        }

        // Try multicast queue
        if let Ok(result) = Self::receive_from_queue_static(&mut self.mcast_queue, buf) {
            return Ok(result);
        }

        Err("No packets available")
    }

    fn receive_from_queue_static(queue: &mut RxQueue, buf: &mut [u8]) -> Result<RxResult, &'static str> {
        if queue.head == queue.tail {
            return Err("Queue empty");
        }

        let desc_offset = queue.head * core::mem::size_of::<RxDescriptor>();
        let desc_ptr = unsafe { queue.base.add(desc_offset) as *const RxDescriptor };
        let desc = unsafe { core::ptr::read_volatile(desc_ptr) };

        if desc.flags & RX_DESC_DONE == 0 {
            return Err("Packet not ready");
        }

        let data_ptr = desc.addr_low as *const u8;
        let length = desc.length as usize;
        let copy_len = core::cmp::min(length, buf.len());

        unsafe {
            core::ptr::copy_nonoverlapping(data_ptr, buf.as_mut_ptr(), copy_len);
        }

        // Advance head
        queue.head = (queue.head + 1) % queue.count;

        Ok(RxResult {
            length: copy_len,
            rssi: desc.rssi,
            rate: desc.rate,
            channel: desc.channel,
            flags: desc.flags,
            timestamp: desc.timestamp,
        })
    }

    pub fn receive_monitor(&mut self, buf: &mut [u8]) -> Result<RxResult, &'static str> {
        Self::receive_from_queue_static(&mut self.monitor_queue, buf)
    }

    pub fn available(&self) -> usize {
        self.queue_depth(&self.main_queue) + self.queue_depth(&self.mcast_queue)
    }

    pub fn available_monitor(&self) -> usize {
        self.queue_depth(&self.monitor_queue)
    }

    fn queue_depth(&self, queue: &RxQueue) -> usize {
        if queue.tail >= queue.head {
            queue.tail - queue.head
        } else {
            queue.count - queue.head + queue.tail
        }
    }

    pub fn handle_rx_done(&mut self) -> usize {
        let mut total = 0;
        total += Self::process_queue_static(&mut self.main_queue);
        total += Self::process_queue_static(&mut self.mcast_queue);
        total += Self::process_queue_static(&mut self.monitor_queue);
        total
    }

    fn process_queue_static(queue: &mut RxQueue) -> usize {
        let mut processed = 0;
        while queue.head != queue.tail {
            let desc_offset = queue.head * core::mem::size_of::<RxDescriptor>();
            let desc_ptr = unsafe { queue.base.add(desc_offset) as *const RxDescriptor };
            let desc = unsafe { core::ptr::read_volatile(desc_ptr) };

            if desc.flags & RX_DESC_DONE == 0 {
                break;
            }

            queue.head = (queue.head + 1) % queue.count;
            processed += 1;
        }
        processed
    }

    pub fn enable_monitor_mode(&mut self, enable: bool) {
        unsafe {
            if enable {
                core::ptr::write_volatile(self.regs.add(RX_MONITOR_CTRL), RX_MONITOR_CTRL_ENABLE);
            } else {
                core::ptr::write_volatile(self.regs.add(RX_MONITOR_CTRL), 0);
            }
        }
    }

    pub fn set_rssi_threshold(&mut self, threshold: i8) {
        unsafe {
            core::ptr::write_volatile(self.regs.add(RX_RSSI_THRESHOLD), threshold as u32);
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct RxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
    rssi: i8,
    rate: u8,
    channel: u8,
    reserved: u8,
    timestamp: u64,
}

#[derive(Debug)]
pub struct RxResult {
    pub length: usize,
    pub rssi: i8,
    pub rate: u8,
    pub channel: u8,
    pub flags: u32,
    pub timestamp: u64,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RxFlags: u32 {
        const VALID = 0x00000001;
        const DONE = 0x80000000;
        const CRC_ERR = 0x00000002;
        const SHORT_PREAMBLE = 0x00000004;
        const OFDM = 0x00000008;
        const CCK = 0x00000010;
        const HT = 0x00000020;
        const VHT = 0x00000040;
        const HE = 0x00000080;
        const EHT = 0x00000100; // WiFi 7
        const AGG = 0x00000200;  // Aggregated
        const BA = 0x00000400;   // Block Ack
        const DECYPTED = 0x00000800;
        const MIC_ERR = 0x00001000;
        const REPLAY = 0x00002000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rx_flags() {
        assert_eq!(RxFlags::VALID.bits(), 0x00000001);
        assert_eq!(RxFlags::DONE.bits(), 0x80000000);
        assert_eq!(RxFlags::CRC_ERR.bits(), 0x00000002);
        assert_eq!(RxFlags::HT.bits(), 0x00000020);
        assert_eq!(RxFlags::VHT.bits(), 0x00000040);
        assert_eq!(RxFlags::HE.bits(), 0x00000080);
        assert_eq!(RxFlags::EHT.bits(), 0x00000100);

        let combined = RxFlags::VALID | RxFlags::HT | RxFlags::AGG;
        assert!(combined.contains(RxFlags::VALID));
        assert!(combined.contains(RxFlags::HT));
        assert!(combined.contains(RxFlags::AGG));
    }

    #[test]
    fn test_rx_descriptor_size() {
        assert_eq!(core::mem::size_of::<RxDescriptor>(), 32);
    }

    #[test]
    fn test_rx_result() {
        let result = RxResult {
            length: 1500,
            rssi: -45,
            rate: 12,
            channel: 36,
            flags: RxFlags::VALID.bits() | RxFlags::HT.bits(),
            timestamp: 1234567890,
        };
        assert_eq!(result.length, 1500);
        assert_eq!(result.rssi, -45);
        assert!(result.flags & RxFlags::VALID.bits() != 0);
    }
}