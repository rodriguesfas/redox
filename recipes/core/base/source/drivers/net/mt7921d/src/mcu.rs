//! MCU (Microcontroller Unit) communication for MT7921

use alloc::vec::Vec;
use alloc::sync::Arc;

pub struct Mcu {
    regs: *mut u32,
    fw: Arc<crate::fw::Firmware>,
    msg_id: u32,
    pending_cmds: Vec<McuCmd>,
}

struct McuCmd {
    id: u32,
    cmd: u32,
    data: Vec<u8>,
    response: Option<Vec<u8>>,
}

impl Mcu {
    pub fn new(regs: *mut u32, fw: alloc::sync::Arc<crate::fw::Firmware>) -> Result<Self, &'static str> {
        log::info!("MT7921 MCU: Initializing");
        
        // Initialize MCU registers
        unsafe {
            // MT7921_MCU_CTRL
            core::ptr::write_volatile(regs.add(0x5000), 0x1);
            // MT7921_MCU_INT_ENABLE
            core::ptr::write_volatile(regs.add(0x5100), 0xFFFFFFFF);
        }
        
        // Send init command to firmware
        // This would normally send a firmware start command
        
        Ok(Self {
            regs,
            fw,
            msg_id: 0,
            pending_cmds: Vec::new(),
        })
    }
    
    pub fn send_cmd(&mut self, cmd: u32, data: &[u8]) -> Result<u32, &'static str> {
        let id = self.msg_id;
        self.msg_id = self.msg_id.wrapping_add(1);
        
        let mcu_cmd = McuCmd {
            id,
            cmd,
            data: data.to_vec(),
            response: None,
        };
        
        self.pending_cmds.push(mcu_cmd);
        
        // Write command to MCU mailbox
        unsafe {
            core::ptr::write_volatile(self.regs.add(0x5200), cmd);
            core::ptr::write_volatile(self.regs.add(0x5204), id);
            // Write data...
        }
        
        // Ring doorbell
        unsafe {
            core::ptr::write_volatile(self.regs.add(0x5004), 0x1);
        }
        
        Ok(id)
    }
    
    pub fn handle_firmware_event(&mut self) {
        // Read event from MCU mailbox
        let event = unsafe { core::ptr::read_volatile(self.regs.add(0x5300)) };
        let id = unsafe { core::ptr::read_volatile(self.regs.add(0x5304)) };
        
        log::trace!("MT7921 MCU: Event {:#x} for cmd {}", event, id);
        
        // Find and complete pending command
        if let Some(idx) = self.pending_cmds.iter().position(|c| c.id == id) {
            let mut cmd = self.pending_cmds.remove(idx);
            cmd.response = Some(vec![event as u8]);
        }
    }
    
    pub fn process_messages(&mut self) {
        // Process any pending MCU messages
        while let Some(event) = self.read_event() {
            log::trace!("MT7921 MCU: Processing event {:#x}", event);
        }
    }
    
    fn read_event(&mut self) -> Option<u32> {
        let status = unsafe { core::ptr::read_volatile(self.regs.add(0x5104)) };
        if status & 0x1 != 0 {
            Some(unsafe { core::ptr::read_volatile(self.regs.add(0x5300)) })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcu_cmd_creation() {
        let cmd = McuCmd {
            id: 1,
            cmd: 0x10,
            data: vec![0x01, 0x02, 0x03],
            response: None,
        };
        assert_eq!(cmd.id, 1);
        assert_eq!(cmd.cmd, 0x10);
        assert_eq!(cmd.data, vec![0x01, 0x02, 0x03]);
        assert!(cmd.response.is_none());
    }

    #[test]
    fn test_mcu_cmd_with_response() {
        let mut cmd = McuCmd {
            id: 1,
            cmd: 0x10,
            data: vec![],
            response: None,
        };
        cmd.response = Some(vec![0x01]);
        assert!(cmd.response.is_some());
        assert_eq!(cmd.response.unwrap(), vec![0x01]);
    }

    #[test]
    fn test_msg_id_wrapping() {
        let mut msg_id: u32 = u32::MAX;
        msg_id = msg_id.wrapping_add(1);
        assert_eq!(msg_id, 0);
    }

    #[test]
    fn test_mcu_commands() {
        // Test MCU command constants
        assert_eq!(0x01, 0x01); // MT7921_MCU_CMD_FW_START
        assert_eq!(0x10, 0x10); // MT7921_MCU_CMD_SCAN
        assert_eq!(0x11, 0x11); // MT7921_MCU_CMD_CONNECT
        assert_eq!(0x20, 0x20); // MT7921_MCU_CMD_SET_CHANNEL
    }
}