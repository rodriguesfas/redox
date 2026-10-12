//! MVM (MAC Virtual Machine) for Intel iwlwifi
//! Handles firmware command/response communication

use alloc::vec::Vec;
use alloc::sync::Arc;
use crate::fw::Firmware;
use crate::mac::Mac;

pub struct Mvm {
    regs: *mut u32,
    fw: Arc<Firmware>,
    msg_id: u32,
    pending_cmds: Vec<MvmCmd>,
    rx_ba_sessions: Vec<BaSession>,
    tx_ba_sessions: Vec<BaSession>,
}

struct MvmCmd {
    id: u32,
    cmd: u32,
    data: Vec<u8>,
    response: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct BaSession {
    sta_id: u8,
    tid: u8,
    ssn: u16,
    buf_size: u16,
    timeout: u16,
}

impl Mvm {
    pub fn new(regs: *mut u32, fw: &Arc<Firmware>, _mac: &mut Mac) -> Result<Self, &'static str> {
        log::info!("iwlwifi MVM: Initializing");

        // Initialize MVM registers
        unsafe {
            // MVM_CTRL
            core::ptr::write_volatile(regs.add(crate::regs::MVM_CTRL), crate::regs::MVM_CTRL_ENABLE);
            // MVM_INT_ENABLE
            core::ptr::write_volatile(regs.add(crate::regs::MVM_INT_ENABLE), 0xFFFFFFFF);
        }

        // Send firmware start command
        Self::send_fw_start_cmd(regs)?;

        Ok(Self {
            regs,
            fw: fw.clone(),
            msg_id: 0,
            pending_cmds: Vec::new(),
            rx_ba_sessions: Vec::new(),
            tx_ba_sessions: Vec::new(),
        })
    }

    fn send_fw_start_cmd(regs: *mut u32) -> Result<(), &'static str> {
        log::info!("iwlwifi MVM: Sending firmware start command");

        unsafe {
            // Write command to MVM mailbox
            core::ptr::write_volatile(regs.add(crate::regs::MVM_CMD), crate::regs::MVM_CMD_FW_START);
            core::ptr::write_volatile(regs.add(crate::regs::MVM_CMD_ID), 0);
            // Ring doorbell
            core::ptr::write_volatile(regs.add(crate::regs::MVM_DOORBELL), 0x1);
        }

        // Wait for firmware ready (with timeout)
        for _ in 0..1000 {
            let status = unsafe { core::ptr::read_volatile(regs.add(crate::regs::MVM_STATUS)) };
            if status & crate::regs::MVM_STATUS_FW_READY != 0 {
                log::info!("iwlwifi MVM: Firmware ready");
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err("Firmware start timeout")
    }

    pub fn send_cmd(&mut self, cmd: u32, data: &[u8]) -> Result<u32, &'static str> {
        let id = self.msg_id;
        self.msg_id = self.msg_id.wrapping_add(1);

        let mvm_cmd = MvmCmd {
            id,
            cmd,
            data: data.to_vec(),
            response: None,
        };

        self.pending_cmds.push(mvm_cmd);

        // Write command to MVM mailbox
        unsafe {
            core::ptr::write_volatile(self.regs.add(crate::regs::MVM_CMD), cmd);
            core::ptr::write_volatile(self.regs.add(crate::regs::MVM_CMD_ID), id);
            // Write data if any
            if !data.is_empty() {
                let data_ptr = self.regs.add(crate::regs::MVM_CMD_DATA) as *mut u8;
                for (i, &byte) in data.iter().enumerate() {
                    core::ptr::write_volatile(data_ptr.add(i), byte);
                }
            }
            // Ring doorbell
            core::ptr::write_volatile(self.regs.add(crate::regs::MVM_DOORBELL), 0x1);
        }

        Ok(id)
    }

    pub fn send_cmd_wait(&mut self, cmd: u32, data: &[u8], timeout_ms: u32) -> Result<Vec<u8>, &'static str> {
        let id = self.send_cmd(cmd, data)?;

        // Wait for response
        for _ in 0..timeout_ms * 1000 {
            self.process_messages();

            if let Some(idx) = self.pending_cmds.iter().position(|c| c.id == id) {
                if let Some(ref resp) = self.pending_cmds[idx].response {
                    return Ok(resp.clone());
                }
            }
            core::hint::spin_loop();
        }

        Err("Command timeout")
    }

    pub fn handle_firmware_event(&mut self) {
        // Read event from MVM mailbox
        let event = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::MVM_EVENT)) };
        let id = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::MVM_EVENT_ID)) };
        let event_data = unsafe {
            let data_ptr = self.regs.add(crate::regs::MVM_EVENT_DATA) as *const u8;
            let mut data = Vec::new();
            for i in 0..64 {
                data.push(core::ptr::read_volatile(data_ptr.add(i)));
            }
            data
        };

        log::trace!("iwlwifi MVM: Event {:#x} for cmd {}", event, id);

        // Find and complete pending command
        if let Some(idx) = self.pending_cmds.iter().position(|c| c.id == id) {
            let mut cmd = self.pending_cmds.remove(idx);
            cmd.response = Some(event_data.clone());
            self.pending_cmds.insert(idx, cmd);
        }

        // Handle async events (notifications)
        self.handle_async_event(event, &event_data);
    }

    fn handle_async_event(&mut self, event: u32, _data: &[u8]) {
        match event {
            crate::regs::MVM_EVENT_RX => {
                log::trace!("iwlwifi MVM: RX notification");
            }
            crate::regs::MVM_EVENT_TX => {
                log::trace!("iwlwifi MVM: TX notification");
            }
            crate::regs::MVM_EVENT_BA => {
                log::trace!("iwlwifi MVM: Block Ack notification");
            }
            crate::regs::MVM_EVENT_SCAN => {
                log::trace!("iwlwifi MVM: Scan notification");
            }
            crate::regs::MVM_EVENT_BEACON => {
                log::trace!("iwlwifi MVM: Beacon notification");
            }
            crate::regs::MVM_EVENT_TDLS => {
                log::trace!("iwlwifi MVM: TDLS notification");
            }
            _ => {
                log::trace!("iwlwifi MVM: Unknown async event {:#x}", event);
            }
        }
    }

    pub fn process_messages(&mut self) {
        // Process any pending MVM messages
        while let Some(event) = self.read_event() {
            log::trace!("iwlwifi MVM: Processing event {:#x}", event);
        }
    }

    fn read_event(&mut self) -> Option<u32> {
        let status = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::MVM_INT_STATUS)) };
        if status & 0x1 != 0 {
            Some(unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::MVM_EVENT)) })
        } else {
            None
        }
    }

    pub fn add_ba_session(&mut self, sta_id: u8, tid: u8, ssn: u16, buf_size: u16) -> Result<(), &'static str> {
        log::info!("iwlwifi MVM: Adding BA session STA={} TID={} SSN={} BUF={}", sta_id, tid, ssn, buf_size);

        let session = BaSession {
            sta_id,
            tid,
            ssn,
            buf_size,
            timeout: 0,
        };

        self.rx_ba_sessions.push(session);
        Ok(())
    }

    pub fn remove_ba_session(&mut self, sta_id: u8, tid: u8) -> Result<(), &'static str> {
        log::info!("iwlwifi MVM: Removing BA session STA={} TID={}", sta_id, tid);

        self.rx_ba_sessions.retain(|s| s.sta_id != sta_id || s.tid != tid);
        self.tx_ba_sessions.retain(|s| s.sta_id != sta_id || s.tid != tid);
        Ok(())
    }

    pub fn start_scan(&mut self, scan_type: ScanType, channels: &[u8]) -> Result<(), &'static str> {
        log::info!("iwlwifi MVM: Starting {:?} scan on {} channels", scan_type, channels.len());

        let mut data = Vec::new();
        data.push(scan_type as u8);
        data.push(channels.len() as u8);
        data.extend_from_slice(channels);

        self.send_cmd(crate::regs::MVM_CMD_SCAN, &data)?;
        Ok(())
    }

    pub fn get_statistics(&mut self) -> Result<StationStats, &'static str> {
        let resp = self.send_cmd_wait(crate::regs::MVM_CMD_GET_STATS, &[], 100)?;

        // Parse statistics response
        Ok(StationStats::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ScanType {
    Passive = 0,
    Active = 1,
    Background = 2,
}

#[derive(Debug, Default)]
pub struct StationStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_retries: u64,
    pub tx_failed: u64,
    pub rx_crc_errors: u64,
    pub signal_dbm: i8,
    pub noise_dbm: i8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mvm_commands() {
        assert_eq!(crate::regs::MVM_CMD_FW_START, 0x01);
        assert_eq!(crate::regs::MVM_CMD_SCAN, 0x10);
        assert_eq!(crate::regs::MVM_CMD_CONNECT, 0x11);
        assert_eq!(crate::regs::MVM_CMD_GET_STATS, 0x40);
    }

    #[test]
    fn test_mvm_events() {
        assert_eq!(crate::regs::MVM_EVENT_RX, 0x100);
        assert_eq!(crate::regs::MVM_EVENT_TX, 0x101);
        assert_eq!(crate::regs::MVM_EVENT_SCAN, 0x103);
    }

    #[test]
    fn test_scan_types() {
        assert_eq!(ScanType::Passive as u8, 0);
        assert_eq!(ScanType::Active as u8, 1);
        assert_eq!(ScanType::Background as u8, 2);
    }

    #[test]
    fn test_station_stats_default() {
        let stats = StationStats::default();
        assert_eq!(stats.tx_packets, 0);
        assert_eq!(stats.rx_packets, 0);
        assert_eq!(stats.signal_dbm, 0);
    }
}