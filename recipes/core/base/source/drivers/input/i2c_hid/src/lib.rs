//! I2C-HID protocol codec (USB HID over I2C) — protocolo do kernel Linux i2c-hid.
//!
//! Modelo: a device anuncia um "HID descriptor" num registrador fixo (0x01),
//! contendo os registradores de comando/dados. Operacoes: power on/off,
//! reset, set idle, get/set report.
//!
//! Testavel sem hardware via o trait `I2cBus` (no-op em mock).

extern crate alloc;
use alloc::vec::Vec;

/// Interface generica (dada pelo Dwd ou outro master I2C).
pub trait I2cBus {
    /// Escreve bytes no escravo 7-bit.
    fn tx(&mut self, addr: u8, data: &[u8]) -> Result<(), &'static str>;
    /// Le N bytes do escravo.
    fn rx(&mut self, addr: u8, out: &mut [u8]) -> Result<(), &'static str>;
    /// Escreve e le (repeated start).
    fn tx_rx(&mut self, addr: u8, tx: &[u8], out: &mut [u8]) -> Result<(), &'static str>;
}

// ---- OpCodes principais (i2c-hid kernel) ----
pub const CMD_GET_REPORT: u8 = 0x02;
pub const CMD_SET_REPORT: u8 = 0x03;
pub const CMD_GET_IDLE: u8 = 0x04;
pub const CMD_SET_IDLE: u8 = 0x05;
pub const CMD_GET_PROTOCOL: u8 = 0x06;
pub const CMD_SET_PROTOCOL: u8 = 0x07;
pub const CMD_RESET: u8 = 0x08;
pub const CMD_SET_POWER: u8 = 0x09;

/// Bits de "power state" do `SET_POWER`
pub const PWR_ON: u8 = 0x00;
pub const PWR_SLEEP: u8 = 0x01;

/// Tipos de report para encodificacao
pub const REPORT_RESERVED: u8 = 0x00;
pub const REPORT_INPUT: u8 = 0x01;
pub const REPORT_OUTPUT: u8 = 0x02;
pub const REPORT_FEATURE: u8 = 0x03;

/// estrutura de descriptor HID (parcial, 30B minimo, big/little endian por campo LE)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidDescriptor {
    pub hiddesc_length: u16,
    pub bcd_version: u16,
    pub report_desc_length: u16,
    pub report_desc_register: u16,
    pub input_register: u16,
    pub max_input_length: u16,
    pub output_register: u16,
    pub max_output_length: u16,
    pub command_register: u16,
    pub data_register: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub version_id: u16,
    pub reserved: u32,
}

impl HidDescriptor {
    pub const MIN_LEN: usize = 30;

    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::MIN_LEN {
            return None;
        }
        let le16 = |i: usize| u16::from_le_bytes([buf[i], buf[i + 1]]);
        let desc = Self {
            hiddesc_length: le16(0),
            bcd_version: le16(2),
            report_desc_length: le16(4),
            report_desc_register: le16(6),
            input_register: le16(8),
            max_input_length: le16(10),
            output_register: le16(12),
            max_output_length: le16(14),
            command_register: le16(16),
            data_register: le16(18),
            vendor_id: le16(20),
            product_id: le16(22),
            version_id: le16(24),
            reserved: u32::from_le_bytes([buf[26], buf[27], buf[28], buf[29]]),
        };
        if desc.hiddesc_length < Self::MIN_LEN as u16 {
            return None;
        }
        Some(desc)
    }
}

fn returnNone() -> Option<HidDescriptor> {
    None
}

/// Device I2C-HID: pequeno wrapper lazy sobre um barramento.
pub struct I2cHidDevice<B: I2cBus> {
    pub bus: B,
    pub addr7: u8,
    pub desc: Option<HidDescriptor>,
}

impl<B: I2cBus> I2cHidDevice<B> {
    pub fn new(bus: B, addr7: u8) -> Self {
        Self {
            bus,
            addr7,
            desc: None,
        }
    }

    /// Le o HID descriptor do device (registrador 0x01).
    pub fn fetch_descriptor(&mut self) -> Result<HidDescriptor, &'static str> {
        let mut buf = [0u8; HidDescriptor::MIN_LEN];
        self.bus.tx_rx(self.addr7, &[0x01], &mut buf)?;
        let desc = HidDescriptor::parse(&buf).ok_or("i2c-hid: desc invalido")?;
        self.desc = Some(desc);
        Ok(desc)
    }

    /// Power on (SET_POWER PWR_ON)
    pub fn power_on(&mut self) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        self.bus.tx(self.addr7, &[
            (desc.command_register & 0xFF) as u8,
            (desc.command_register >> 8) as u8,
            (CMD_SET_POWER << 4) | PWR_ON,
            0x00,
        ])
    }

    /// Power off (SLEEP)
    pub fn power_off(&mut self) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        self.bus.tx(self.addr7, &[
            (desc.command_register & 0xFF) as u8,
            (desc.command_register >> 8) as u8,
            (CMD_SET_POWER << 4) | PWR_SLEEP,
            0x00,
        ])
    }

    /// Reset (sem argumentos)
    pub fn reset(&mut self) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        self.bus.tx(self.addr7, &[
            (desc.command_register & 0xFF) as u8,
            (desc.command_register >> 8) as u8,
            CMD_RESET << 4,
            0x00,
        ])
    }

    /// Set idle (0 = nao dormir; >0 = segundos)
    pub fn set_idle(&mut self, idle_sec: u8) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        self.bus.tx(self.addr7, &[
            (desc.command_register & 0xFF) as u8,
            (desc.command_register >> 8) as u8,
            CMD_SET_IDLE << 4,
            idle_sec,
            0x00,
        ])
    }

    /// Le um Input Report (dados do touchpad) — BF re vai para input_register.
    pub fn read_input(&mut self, buf: &mut [u8]) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        let max = desc.max_input_length as usize;
        if buf.len() < 2 + max {
            return Err("i2c-hid: buffer pequeno");
        }
        self.bus.tx_rx(self.addr7, &[(desc.input_register & 0xFF) as u8], buf)?;
        Ok(())
    }

    /// Get Report (opc: busca dados sob demanda)
    pub fn get_report(&mut self, report_id: u8, report_type: u8, buf: &mut [u8]) -> Result<(), &'static str> {
        let desc = self.desc.ok_or("i2c-hid: sem descriptor")?;
        let mut tx = Vec::with_capacity(6);
        tx.push((desc.command_register & 0xFF) as u8);
        tx.push((desc.command_register >> 8) as u8);
        tx.push((CMD_GET_REPORT << 4) | report_type);
        tx.push(report_id);
        self.bus.tx_rx(self.addr7, &tx, buf)
    }
}

/// Touch frame parseado (p/ touchpad/screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchSlot {
    pub x: u16,
    pub y: u16,
    pub id: u8,   // slot/finger id
    pub valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Barramento falso que grava transactions e responde com dados preenchidos.
    struct FakeBus {
        writes: Vec<(u8, Vec<u8>)>,
        responses: Vec<Vec<u8>>,
    }

    impl FakeBus {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                responses: Vec::new(),
            }
        }
        fn push_response(&mut self, bytes: Vec<u8>) {
            self.responses.push(bytes);
        }
    }

    impl I2cBus for FakeBus {
        fn tx(&mut self, addr: u8, data: &[u8]) -> Result<(), &'static str> {
            self.writes.push((addr, data.to_vec()));
            Ok(())
        }
        fn rx(&mut self, _addr: u8, out: &mut [u8]) -> Result<(), &'static str> {
            if !self.responses.is_empty() {
                let r = self.responses.remove(0);
                out[..r.len()].copy_from_slice(&r);
            }
            Ok(())
        }
        fn tx_rx(&mut self, addr: u8, tx: &[u8], out: &mut [u8]) -> Result<(), &'static str> {
            self.tx(addr, tx)?;
            self.rx(addr, out)
        }
    }

    fn mk_descriptor(vendor: u16, product: u16) -> HidDescriptor {
        HidDescriptor {
            hiddesc_length: 30,
            bcd_version: 0x0100,
            report_desc_length: 128,
            report_desc_register: 0x02,
            input_register: 0x15,
            max_input_length: 12,
            output_register: 0x16,
            max_output_length: 4,
            command_register: 0x22,
            data_register: 0x23,
            vendor_id: vendor,
            product_id: product,
            version_id: 0x0100,
            reserved: 0,
        }
    }

    /// fake descriptor bytes (point Debian style) com le16 encoding
    fn desc_bytes(desc: &HidDescriptor) -> Vec<u8> {
        let mut b = Vec::with_capacity(30);
        let push = |b: &mut Vec<u8>, v: u16| {
            b.push((v & 0xFF) as u8);
            b.push((v >> 8) as u8);
        };
        push(&mut b, desc.hiddesc_length);
        push(&mut b, desc.bcd_version);
        push(&mut b, desc.report_desc_length);
        push(&mut b, desc.report_desc_register);
        push(&mut b, desc.input_register);
        push(&mut b, desc.max_input_length);
        push(&mut b, desc.output_register);
        push(&mut b, desc.max_output_length);
        push(&mut b, desc.command_register);
        push(&mut b, desc.data_register);
        push(&mut b, desc.vendor_id);
        push(&mut b, desc.product_id);
        push(&mut b, desc.version_id);
        b.extend_from_slice(&desc.reserved.to_le_bytes());
        b
    }

    #[test]
    fn desc_parse_roundtrip() {
        let orig = mk_descriptor(0x093A, 0x3848);
        let bytes = desc_bytes(&orig);
        let parsed = HidDescriptor::parse(&bytes).unwrap();
        assert_eq!(parsed.vendor_id, 0x093A);
        assert_eq!(parsed.product_id, 0x3848);
        assert_eq!(parsed.command_register, 0x22);
        assert_eq!(parsed.hiddesc_length, 30);
    }

    #[test]
    fn desc_rejeita_truncado_e_tamanho_errado() {
        let d = mk_descriptor(0x093A, 0x3848);
        // curto de mais
        let bytes = desc_bytes(&d);
        assert!(HidDescriptor::parse(&bytes[..29]).is_none());
        // tamanho declarando menos que o minimo
        let mut short = bytes.clone();
        short[0] = 29;
        short[1] = 0;
        assert!(HidDescriptor::parse(&short).is_none());
    }

    #[test]
    fn bus_transaction_e_desc_codificadas() {
        let mut bus = FakeBus::new();
        let d = mk_descriptor(0x093A, 0x3848);
        bus.push_response(desc_bytes(&d));
        let mut dev = I2cHidDevice::new(bus, 0x20);
        let desc = dev.fetch_descriptor().unwrap();
        assert_eq!(desc.vendor_id, 0x093A);

        // Writes: primeiro byte de comando do fetch do descriptor
        let bus = dev.bus;
        assert_eq!(bus.writes.len(), 1);
        assert_eq!(bus.writes[0].1[0], 0x01);
    }

    #[test]
    fn power_on_reset_set_idle_ordem_correta() {
        let bus = FakeBus::new();
        let mut dev = I2cHidDevice::new(bus, 0x20);
        // Descriptor e "bus.tx" so testa formato: SET_POWER_ON -> 09|00 = 0x90
        let d = mk_descriptor(0x093A, 0x3848);
        dev.desc = Some(d);
        dev.power_on().unwrap();
        dev.reset().unwrap();
        dev.set_idle(7).unwrap();
        let writes = &dev.bus.writes;
        assert_eq!(writes[0].1[2], (CMD_SET_POWER << 4) | PWR_ON);
        assert_eq!(writes[1].1[2], CMD_RESET << 4);
        assert_eq!(writes[2].1[2], CMD_SET_IDLE << 4);
        assert_eq!(writes[2].1[3], 7);
        assert_eq!(writes[2].1[4], 0);
        // hdw descriptor register LE escaping check
        assert_eq!(writes[2].1[0], 0x22 & 0xFF);
        assert_eq!(writes[2].1[1], (0x22 >> 8) as u8);
    }

    #[test]
    fn get_report_encodes_correctly() {
        let bus = FakeBus::new();
        let mut dev = I2cHidDevice::new(bus, 0x20);
        let d = mk_descriptor(0x093A, 0x3848);
        dev.desc = Some(d);
        let mut out = [0u8; 4];
        // sem resposta real, apenas validamos o formato do pedido
        let _ = dev.get_report(5, REPORT_FEATURE, &mut out);
        let w = &dev.bus.writes[0].1;
        assert_eq!(w[0], 0x22);
        assert_eq!(w[1], 0x00);
        assert_eq!(w[2], (CMD_GET_REPORT << 4) | REPORT_FEATURE);
        assert_eq!(w[3], 5); // report id
    }
}

/// Estado de energia
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    On,
    Sleep,
}

// ============================================================
// Touch report parsing (moved from i2c-hidd to keep it host-testable)
// ============================================================

/// Pacote de toque (formato touchpad multiponto comum — PixArt/Elan-like).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchPoint {
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub contact: bool,
}

/// Interpreta o bloco de contato: [report_id][tip/flags][id][x lo][x hi][y lo][y hi]
pub fn parse_contact_block(buf: &[u8]) -> Option<TouchPoint> {
    if buf.len() < 8 {
        return None;
    }
    let contact = buf[2] & 0x01 != 0;
    let id = buf[3];
    let x = u16::from_le_bytes([buf[4], buf[5]]);
    let y = u16::from_le_bytes([buf[6], buf[7]]);
    Some(TouchPoint { id, x, y, contact })
}

#[cfg(test)]
mod touch_tests {
    use super::*;

    #[test]
    fn contact_block_valid() {
        let tp = parse_contact_block(&[1, 1, 1, 2, 0x10, 0x00, 0x20, 0x00]).unwrap();
        assert_eq!(tp.id, 2);
        assert_eq!(tp.x, 16);
        assert_eq!(tp.y, 32);
        assert!(tp.contact);
    }

    #[test]
    fn contact_block_short_fails() {
        assert!(parse_contact_block(&[1u8, 2, 3]).is_none());
    }

    #[test]
    fn no_contact_marks_up() {
        let tp = parse_contact_block(&[1, 1, 0, 2, 0x10, 0x00, 0x20, 0x00]).unwrap();
        assert!(!tp.contact);
    }
}
