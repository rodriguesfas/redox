//! EEPROM reading for MT7921

pub struct Eeprom {
    data: [u8; 512],
}

impl Eeprom {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("MT7921 EEPROM: Reading");
        
        // Read EEPROM via MAC register interface
        let mut data = [0u8; 512];
        
        // MT7921_EEPROM_CTRL
        unsafe {
            for i in 0..256 {
                core::ptr::write_volatile(regs.add(0x6000), i as u32);
                core::ptr::write_volatile(regs.add(0x6004), 0x1); // Read command
                core::hint::spin_loop();
                let val = core::ptr::read_volatile(regs.add(0x6008));
                data[i * 2] = (val & 0xFF) as u8;
                data[i * 2 + 1] = ((val >> 8) & 0xFF) as u8;
            }
        }
        
        Ok(Self { data })
    }
    
    pub fn mac_address(&self) -> [u8; 6] {
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&self.data[0x04..0x0A]);
        mac
    }
    
    pub fn country_code(&self) -> [u8; 2] {
        let mut cc = [0u8; 2];
        cc.copy_from_slice(&self.data[0x10..0x12]);
        cc
    }
    
    pub fn xtal_freq(&self) -> u32 {
        // Crystal frequency in kHz
        u32::from_le_bytes([self.data[0x20], self.data[0x21], self.data[0x22], self.data[0x23]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eeprom_struct() {
        let eeprom = Eeprom { data: [0u8; 512] };
        assert_eq!(eeprom.data.len(), 512);
    }

    #[test]
    fn test_mac_address_offset() {
        let mut data = [0u8; 512];
        data[0x04..0x0A].copy_from_slice(&[0x00, 0x0C, 0xE7, 0x12, 0x34, 0x56]);
        let eeprom = Eeprom { data };
        assert_eq!(eeprom.mac_address(), [0x00, 0x0C, 0xE7, 0x12, 0x34, 0x56]);
    }

    #[test]
    fn test_country_code_offset() {
        let mut data = [0u8; 512];
        data[0x10..0x12].copy_from_slice(b"US");
        let eeprom = Eeprom { data };
        assert_eq!(&eeprom.country_code(), b"US");
    }

    #[test]
    fn test_xtal_freq() {
        let mut data = [0u8; 512];
        data[0x20..0x24].copy_from_slice(&[0x40, 0x42, 0x0F, 0x00]); // 1000000 = 0x000F4240
        let eeprom = Eeprom { data };
        assert_eq!(eeprom.xtal_freq(), 1_000_000);
    }
}