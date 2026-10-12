//! RX (Reception) path for Realtek rtw88

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
        log::info!("rtw88 RX: Initializing reception engine");

        let main_queue = RxQueue {
            base: unsafe { regs.add(REG_RXQ_BASE) },
            count: 512,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Main,
        };

        let monitor_queue = RxQueue {
            base: unsafe { regs.add(REG_RXQ_BASE + 0x800) },
            count: 128,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Monitor,
        };

        let mcast_queue = RxQueue {
            base: unsafe { regs.add(REG_RXQ_BASE + 0x1000) },
            count: 128,
            head: 0,
            tail: 0,
            queue_type: RxQueueType::Multicast,
        };

        // Enable RX DMA
        unsafe {
            core::ptr::write_volatile(regs.add(REG_RXQ_CTRL), RXQ_CTRL_EN);
            // Configure RSSI threshold
            core::ptr::write_volatile(regs.add(REG_RF_RSSI), 0xFF);
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

        if desc.flags & RX_DESC_OWN != 0 {
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
        Self::queue_depth_static(&self.main_queue) + Self::queue_depth_static(&self.mcast_queue)
    }

    pub fn available_monitor(&self) -> usize {
        Self::queue_depth_static(&self.monitor_queue)
    }

    fn queue_depth_static(queue: &RxQueue) -> usize {
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

            if desc.flags & RX_DESC_OWN != 0 {
                break;
            }

            queue.head = (queue.head + 1) % queue.count;
            processed += 1;
        }
        processed
    }

    pub fn enable_monitor_mode(&mut self, enable: bool) {
        // Realtek monitor mode is configured via MAC register
        unsafe {
            if enable {
                core::ptr::write_volatile(self.regs.add(REG_RCR), 
                    RXQ_CTRL_EN | 0x00020000); // Enable promiscuous
            } else {
                core::ptr::write_volatile(self.regs.add(REG_RCR), RXQ_CTRL_EN);
            }
        }
    }

    pub fn set_rssi_threshold(&mut self, threshold: i8) {
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_RF_RSSI), threshold as u32);
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
        const OWN = 0x80000000;
        const EOR = 0x40000000;
        const FS = 0x20000000;
        const LS = 0x10000000;
        const CRC32 = 0x08000000;
        const ICV_ERR = 0x04000000;
        const DEC_ERR = 0x02000000;
        const CRC_ERR = 0x01000000;
        const MIC_ERR = 0x00800000;
        const REPLAY = 0x00400000;
        const PHYSTS = 0x00008000;
        const SW_DEC = 0x00004000;
        const HW_DEC = 0x00002000;
        const ENC_TYPE = 0x00001F00;
        const AMSDU = 0x00000080;
        const AMPDU = 0x00000040;
        const UC = 0x00000020;
        const MC = 0x00000010;
        const BC = 0x00000008;
        const MY_BSSID = 0x00000004;
        const DATA_RATE = 0x00000003;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rx_flags() {
        assert_eq!(RxFlags::OWN.bits(), 0x80000000);
        assert_eq!(RxFlags::EOR.bits(), 0x40000000);
        assert_eq!(RxFlags::FS.bits(), 0x20000000);
        assert_eq!(RxFlags::LS.bits(), 0x10000000);
        assert_eq!(RxFlags::CRC32.bits(), 0x08000000);
        assert_eq!(RxFlags::PHYSTS.bits(), 0x00008000);

        let combined = RxFlags::OWN | RxFlags::FS | RxFlags::LS;
        assert!(combined.contains(RxFlags::OWN));
        assert!(combined.contains(RxFlags::FS));
        assert!(combined.contains(RxFlags::LS));
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
            flags: RxFlags::OWN.bits() | RxFlags::FS.bits(),
            timestamp: 1234567890,
        };
        assert_eq!(result.length, 1500);
        assert_eq!(result.rssi, -45);
        assert!(result.flags & RxFlags::OWN.bits() != 0);
    }
}