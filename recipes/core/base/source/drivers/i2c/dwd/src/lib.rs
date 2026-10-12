//! Synopsys DesignWare I2C master driver (barramento, MMIO).
//!
//! Implementa o mínimo para falar com um cliente: enable/disable,
//! set_target (7 bits), write byte, read byte com stop/restart.

pub mod regs;

extern crate alloc;

use alloc::vec::Vec;
use regs::*;

pub struct Dwd {
    regs: *mut u32,
}

impl Dwd {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("dwd/i2c: init DesignWare I2C master");

        let mut s = Self { regs };

        // Disable para configurar
        s.disable();

        // 7-bit addr, master mode, fast mode (400k), restart enable
        let con = CON_MASTER | CON_SPEED_FS | CON_ADDR_7BIT | CON_RESTART_EN | CON_SLAVE_DISABLE;
        unsafe { core::ptr::write_volatile(s.regs.add(REG_CON), con) };

        // Fast mode SCL (400kHz) — valores conservadores p/ bring-up
        unsafe {
            core::ptr::write_volatile(s.regs.add(REG_FS_SCL_HCNT), 50);
            core::ptr::write_volatile(s.regs.add(REG_FS_SCL_LCNT), 50);
        }

        s.enable();

        Ok(s)
    }

    pub fn enable(&mut self) {
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_ENABLE), 1);
        }
    }

    pub fn disable(&mut self) {
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_ENABLE), 0);
        }
    }

    pub fn set_target(&mut self, addr7: u8) -> Result<(), &'static str> {
        if addr7 > 0x7F {
            return Err("i2c 7-bit only");
        }
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_TAR), addr7 as u32);
        }
        Ok(())
    }

    /// Escreve bytes no cliente (start automatico, stop no ultimo).
    pub fn master_tx(&mut self, addr7: u8, data: &[u8]) -> Result<(), &'static str> {
        self.set_target(addr7)?;

        for (i, &b) in data.iter().enumerate() {
            // aguarda TX FIFO nao cheia (TFNF=1 = tem espaco)
            self.wait_mask(STATUS_TFNF, true)?;

            let mut cmd = b as u32;
            if i == data.len() - 1 {
                cmd |= DATA_CMD_STOP; // stop no ultimo
            }
            unsafe {
                core::ptr::write_volatile(self.regs.add(REG_DATA_CMD), cmd);
            }
        }
        // espera esvaziar a FIFO (stop emitido; master fica idle -> MST_ACTIVITY=0)
        self.wait_mask(STATUS_MST_ACTIVITY, false)?;
        Ok(())
    }

    /// Le N bytes do cliente (write de com.request -> auto-read).
    pub fn master_rx(&mut self, addr7: u8, out: &mut [u8]) -> Result<(), &'static str> {
        self.set_target(addr7)?;

        for i in 0..out.len() {
            self.wait_mask(STATUS_TFNF, true)?;

            let mut cmd = DATA_CMD_READ;
            if i == out.len() - 1 {
                cmd |= DATA_CMD_STOP;
            }
            unsafe { core::ptr::write_volatile(self.regs.add(REG_DATA_CMD), cmd) };

            // aguarda dados na RX FIFO
            self.wait_mask(STATUS_RFNE, true)?;

            out[i] = unsafe { core::ptr::read_volatile(self.regs.add(REG_DATA_CMD)) as u8 };
        }
        Ok(())
    }

    /// Escreve um set de registros: [reg][payload...] (tipo i2c-hid).
    pub fn write_reg(&mut self, addr7: u8, reg: [u8; 2], data: &[u8]) -> Result<(), &'static str> {
        let mut buf = Vec::with_capacity(2 + data.len());
        buf.extend_from_slice(&reg);
        buf.extend_from_slice(data);
        self.master_tx(addr7, &buf)
    }

    /// Indicativo de atividade e status
    pub fn is_in_transaction(&self) -> bool {
        unsafe { core::ptr::read_volatile(self.regs.add(REG_STATUS)) & STATUS_MST_ACTIVITY != 0 }
    }

    /// Espera um bit ficar como queremos (true=1, false=0), com timeout.
    /// Espera o bit ficar com o valor pedido (true=1, false=0), com timeout.
    fn wait_mask(&mut self, mask: u32, up: bool) -> Result<(), &'static str> {
        let mut spins = 0usize;
        loop {
            let status = unsafe { core::ptr::read_volatile(self.regs.add(REG_STATUS)) };
            let is_set = (status & mask) != 0;
            if is_set == up {
                break;
            }
            spins += 1;
            if spins > 1_000_000 {
                return Err("i2c: timeout");
            }
            core::hint::spin_loop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_bar_full() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = alloc::vec![0u32; 0x100].into_boxed_slice();
        let p = v.as_mut_ptr();
        (v, p)
    }

    #[test]
    fn init_e_desliga() {
        let (_b, regs) = fake_bar_full();
        let mut i2c = Dwd::new(regs).unwrap();

        // con escrito correto? (0x67)
        let con = unsafe { core::ptr::read_volatile(regs.add(REG_CON)) };
        assert_eq!(con & (CON_MASTER | CON_SPEED_FS | CON_ADDR_7BIT), CON_MASTER | CON_SPEED_FS | CON_ADDR_7BIT);

        i2c.disable();
        let en = unsafe { core::ptr::read_volatile(regs.add(REG_ENABLE)) };
        assert_eq!(en, 0);
    }

    #[test]
    fn master_tx_escreve_datacmd_e_stop_no_ultimo() {
        let (_b, regs) = fake_bar_full();
        let mut i2c = Dwd::new(regs).unwrap();

        // STATUS.TFNF = 1 sempre (fake: simula FIFO livre)
        unsafe { core::ptr::write_volatile(regs.add(REG_STATUS), STATUS_TFNF) };

        i2c.master_tx(0x20, &[0xAA, 0xBB]).unwrap();
        datacmd_last_checks(regs);
    }

    #[test]
    fn master_tx_stop_e_restart_bits() {
        let (_b, regs) = fake_bar_full();
        let mut i2c = Dwd::new(regs).unwrap();
        unsafe { core::ptr::write_volatile(regs.add(REG_STATUS), STATUS_TFNF) };

        i2c.master_tx(0x30, &[1, 2, 3]).unwrap();
        datacmd_last_checks(regs);
    }

    fn datacmd_last_checks(regs: *mut u32) {
        let datacmd = unsafe { core::ptr::read_volatile(regs.add(REG_DATA_CMD)) };
        assert_eq!(datacmd & DATA_CMD_STOP, DATA_CMD_STOP); // stop no ultimo byte
        assert_eq!(datacmd & DATA_CMD_READ, 0); // TX, nao RX
    }

    #[test]
    fn write_reg_formato_correto() {
        let (_b, regs) = fake_bar_full();
        let mut i2c = Dwd::new(regs).unwrap();
        unsafe { core::ptr::write_volatile(regs.add(REG_STATUS), STATUS_TFNF) };

        i2c.write_reg(0x40, [0x00, 0x22], &[0x99]).unwrap();
        // DATACMD recebe a sequencia reg-hi/reg-lo/data/... = ultimo foi 0x99 com stop
        let datacmd = unsafe { core::ptr::read_volatile(regs.add(REG_DATA_CMD)) };
        assert_eq!(datacmd & (DATA_CMD_STOP | DATA_CMD_RESTART), DATA_CMD_STOP);
    }

    #[test]
    /// O falso BAR nao consegue dividir os dois sentidos no mesmo registrador
    /// (destino de escrita == byte de leitura). Validamos o lado do comando:
    /// ultima escrita = READ|STOP.
    fn master_rx_emite_read_stop_no_ultimo() {
        let (_b, regs) = fake_bar_full();
        let mut i2c = Dwd::new(regs).unwrap();
        unsafe { core::ptr::write_volatile(regs.add(REG_STATUS), STATUS_TFNF | STATUS_RFNE) };
        let mut out = [0u8; 2];
        i2c.master_rx(0x51, &mut out).unwrap();
        let datacmd = unsafe { core::ptr::read_volatile(regs.add(REG_DATA_CMD)) };
        // depois do rx: ultimo comando de leitura tinha READ+STOP
        assert_eq!(datacmd & (DATA_CMD_READ | DATA_CMD_STOP), DATA_CMD_READ | DATA_CMD_STOP);
    }
}
