//! BCM2711 GENET v5 Ethernet driver (Raspberry Pi 4) — TX path.
//!
//! Faithful port of U-Boot's `drivers/net/bcmgenet.c`, which is known to
//! work on the Raspberry Pi 4. Registers only; packet buffers live in
//! physically-contiguous uncacheable DMA memory provided by `common::dma`.
//!
//! RX is intentionally not enabled yet (phase M2): enabling RDMA without a
//! fully initialized RX ring would let the hardware DMA into random memory.

use std::time::{Duration, Instant};

use common::dma::Dma;

// ---------------------------------------------------------------------------
// Register offsets (ARM view base 0xfd580000)
// ---------------------------------------------------------------------------

const SYS_REV_CTRL: usize = 0x0000;
const SYS_PORT_CTRL: usize = 0x0004;
const SYS_RBUF_FLUSH_CTRL: usize = 0x0008;

const PORT_MODE_EXT_GPHY: u32 = 3;

const EXT_RGMII_OOB_CTRL: usize = 0x008C; // GENET_EXT_OFF(0x80) + 0x0C
const RGMII_LINK: u32 = 1 << 4;
const OOB_DISABLE: u32 = 1 << 5;
const RGMII_MODE_EN: u32 = 1 << 6;
const ID_MODE_DIS: u32 = 1 << 16;

const RBUF_CTRL: usize = 0x0300;
const RBUF_TBUF_SIZE_CTRL: usize = 0x03B4;
const RBUF_ALIGN_2B: u32 = 1 << 1;

const UMAC_CMD: usize = 0x0808;
const UMAC_MAC0: usize = 0x080C;
const UMAC_MAC1: usize = 0x0810;
const UMAC_MAX_FRAME_LEN: usize = 0x0814;
const UMAC_MIB_CTRL: usize = 0x0D80;
const MDIO_CMD: usize = 0x0E14;
const UMAC_TX_FLUSH: usize = 0x0B34;

const CMD_TX_EN: u32 = 1 << 0;
const CMD_RX_EN: u32 = 1 << 1;
const CMD_SW_RESET: u32 = 1 << 13;
const CMD_LCL_LOOP_EN: u32 = 1 << 15;
const CMD_SPEED_SHIFT: u32 = 2;
const CMD_SPEED_MASK: u32 = 3;

const UMAC_SPEED_10: u32 = 0;
const UMAC_SPEED_100: u32 = 1;
const UMAC_SPEED_1000: u32 = 2;

const MIB_RESET_RX: u32 = 1 << 0;
const MIB_RESET_RUNT: u32 = 1 << 1;
const MIB_RESET_TX: u32 = 1 << 2;

const MDIO_START_BUSY: u32 = 1 << 29;
const MDIO_READ_FAIL: u32 = 1 << 28;
const MDIO_RD: u32 = 2 << 26;
const MDIO_WR: u32 = 1 << 26;
const MDIO_PMD_SHIFT: u32 = 21;
const MDIO_REG_SHIFT: u32 = 16;

// Body(1500) + EH(14) + VLAN(4) + BRCMTAG(6) + FCS(4) + pad(8) = 1536
const ENET_MAX_MTU_SIZE: u32 = 1536;

// ---------------------------------------------------------------------------
// DMA (TX only)
// ---------------------------------------------------------------------------

const GENET_TX_OFF: usize = 0x4000;
const TOTAL_DESCS: usize = 256;
const DMA_DESC_SIZE: usize = 12;
const GENET_TDMA_REG_OFF: usize = GENET_TX_OFF + TOTAL_DESCS * DMA_DESC_SIZE;
const DEFAULT_Q: usize = 0x10;
const DMA_RING_SIZE: usize = 0x40;
const DMA_RINGS_SIZE: usize = DMA_RING_SIZE * (DEFAULT_Q + 1);

const TDMA_RING_BASE: usize = GENET_TDMA_REG_OFF + DEFAULT_Q * DMA_RING_SIZE;
const TDMA_CTRL_BASE: usize = GENET_TDMA_REG_OFF + DMA_RINGS_SIZE;

const DMA_RING_CFG: usize = 0x00;
const DMA_CTRL: usize = 0x04;
const DMA_SCB_BURST_SIZE: usize = 0x0C;
const DMA_READ_PTR: usize = 0x00;
const DMA_CONS_INDEX: usize = 0x08;
const DMA_PROD_INDEX: usize = 0x0C;
const DMA_RING_BUF_SIZE: usize = 0x10;
const DMA_START_ADDR: usize = 0x14;
const DMA_END_ADDR: usize = 0x1C;
const DMA_MBUF_DONE_THRESH: usize = 0x24;
const DMA_FLOW_PERIOD: usize = 0x28;
const DMA_WRITE_PTR: usize = 0x2C;

const DMA_DESC_LENGTH_STATUS: usize = 0x00;
const DMA_DESC_ADDRESS_LO: usize = 0x04;
const DMA_DESC_ADDRESS_HI: usize = 0x08;

const DMA_EN: u32 = 1 << 0;
const DMA_RING_BUF_EN_SHIFT: u32 = 1;
const DMA_MAX_BURST_LENGTH: u32 = 0x8;
const DMA_BUFLENGTH_SHIFT: u32 = 16;
const DMA_TX_APPEND_CRC: u32 = 0x0040;
const DMA_TX_QTAG_SHIFT: u32 = 7;
const DMA_SOP: u32 = 0x2000;
const DMA_EOP: u32 = 0x4000;

const RX_BUF_LENGTH: usize = 2048;
const TX_BUF_LENGTH: usize = 2048;

// ---------------------------------------------------------------------------

fn rd32(base: usize, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

fn wr32(base: usize, off: usize, val: u32) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn mdio_read(base: usize, phy: u32, reg: u32) -> u16 {
    wr32(base, MDIO_CMD, MDIO_RD | (phy << MDIO_PMD_SHIFT) | (reg << MDIO_REG_SHIFT));
    wr32(base, MDIO_CMD, rd32(base, MDIO_CMD) | MDIO_START_BUSY);
    let deadline = Instant::now() + Duration::from_millis(20);
    while rd32(base, MDIO_CMD) & MDIO_START_BUSY != 0 {
        if Instant::now() > deadline {
            return 0;
        }
        sleep_ms(1);
    }
    let val = rd32(base, MDIO_CMD);
    if val & MDIO_READ_FAIL != 0 {
        return 0;
    }
    (val & 0xFFFF) as u16
}

fn mdio_write(base: usize, phy: u32, reg: u32, value: u16) {
    wr32(
        base,
        MDIO_CMD,
        MDIO_WR | (phy << MDIO_PMD_SHIFT) | (reg << MDIO_REG_SHIFT) | (value as u32 & 0xFFFF),
    );
    wr32(base, MDIO_CMD, rd32(base, MDIO_CMD) | MDIO_START_BUSY);
    let deadline = Instant::now() + Duration::from_millis(20);
    while rd32(base, MDIO_CMD) & MDIO_START_BUSY != 0 {
        if Instant::now() > deadline {
            return;
        }
        sleep_ms(1);
    }
}

// ---------------------------------------------------------------------------

pub struct Genet {
    base: usize,
    mac: [u8; 6],
    phyaddr: u32,
    tx_buf: Dma<[u8]>,
    tx_index: usize,
    prod_index: u32,
    speed: u32,
}

impl Genet {
    pub fn new(base: usize, mac: [u8; 6], phyaddr: u32) -> Result<Self, &'static str> {
        let tx_buf = Dma::<[u8]>::zeroed_slice(TX_BUF_LENGTH)
            .map_err(|_| "dma alloc tx buf failed")?;
        let tx_buf = unsafe { tx_buf.assume_init() };
        Ok(Self {
            base,
            mac,
            phyaddr,
            tx_buf,
            tx_index: 0,
            prod_index: 0,
            speed: 0,
        })
    }

    pub fn rev(base: usize) -> u32 {
        rd32(base, SYS_REV_CTRL)
    }

    /// Full bring-up: UMAC reset, MAC address, TX DMA ring, PHY autoneg,
    /// RGMII link setup and TX enable. `log(stage, msg)` persists progress.
    pub fn init(&mut self, log: &dyn Fn(u32, &str)) -> Result<u32, &'static str> {
        let base = self.base;

        // --- interface mode: external Gigabit PHY (RGMII) ---
        log(10, "iface set");
        wr32(base, SYS_PORT_CTRL, PORT_MODE_EXT_GPHY);

        // --- UMAC reset (U-Boot bcmgenet_umac_reset) ---
        log(11, "umac reset");
        let mut reg = rd32(base, SYS_RBUF_FLUSH_CTRL);
        reg |= 1 << 1;
        wr32(base, SYS_RBUF_FLUSH_CTRL, reg);
        sleep_ms(10);
        reg &= !(1 << 1);
        wr32(base, SYS_RBUF_FLUSH_CTRL, reg);
        sleep_ms(10);
        wr32(base, SYS_RBUF_FLUSH_CTRL, 0);
        sleep_ms(10);

        wr32(base, UMAC_CMD, 0);
        wr32(base, UMAC_CMD, CMD_SW_RESET | CMD_LCL_LOOP_EN);
        sleep_ms(2);
        wr32(base, UMAC_CMD, 0);

        wr32(base, UMAC_MIB_CTRL, MIB_RESET_RX | MIB_RESET_TX | MIB_RESET_RUNT);
        wr32(base, UMAC_MIB_CTRL, 0);
        wr32(base, UMAC_MAX_FRAME_LEN, ENET_MAX_MTU_SIZE);

        let reg = rd32(base, RBUF_CTRL);
        wr32(base, RBUF_CTRL, reg | RBUF_ALIGN_2B);
        wr32(base, RBUF_TBUF_SIZE_CTRL, 1);

        // --- MAC address ---
        log(12, "mac addr");
        let reg = (self.mac[0] as u32) << 24
            | (self.mac[1] as u32) << 16
            | (self.mac[2] as u32) << 8
            | (self.mac[3] as u32);
        wr32(base, UMAC_MAC0, reg);
        let reg = (self.mac[4] as u32) << 8 | (self.mac[5] as u32);
        wr32(base, UMAC_MAC1, reg);

        // --- disable TX DMA + flush TX queues ---
        log(13, "dma disable");
        self.disable_dma();

        // --- TX ring ---
        log(14, "tx ring");
        self.tx_ring_init();

        // --- enable TX DMA ---
        log(15, "dma enable");
        let dma_ctrl = (1 << (DEFAULT_Q as u32 + DMA_RING_BUF_EN_SHIFT)) | DMA_EN;
        wr32(base, TDMA_CTRL_BASE + DMA_CTRL, dma_ctrl);

        // --- PHY ---
        log(16, "phy reset");
        let speed = self.phy_init()?;
        self.speed = speed;
        log(17, &format!("link speed {}", speed));

        // --- RGMII link + MAC speed ---
        self.adjust_link(speed);

        // --- enable TX ---
        log(18, "tx enable");
        let cmd = rd32(base, UMAC_CMD);
        wr32(base, UMAC_CMD, (cmd & !(CMD_SPEED_MASK << CMD_SPEED_SHIFT))
            | (speed_code(speed) << CMD_SPEED_SHIFT)
            | CMD_TX_EN);

        log(19, "init done");
        Ok(speed)
    }

    fn disable_dma(&self) {
        let base = self.base;
        let v = rd32(base, TDMA_CTRL_BASE + DMA_CTRL);
        wr32(base, TDMA_CTRL_BASE + DMA_CTRL, v & !DMA_EN);
        wr32(base, UMAC_TX_FLUSH, 1);
        sleep_ms(10);
        wr32(base, UMAC_TX_FLUSH, 0);
    }

    fn tx_ring_init(&mut self) {
        let base = self.base;
        wr32(base, TDMA_CTRL_BASE + DMA_SCB_BURST_SIZE, DMA_MAX_BURST_LENGTH);

        wr32(base, TDMA_RING_BASE + DMA_START_ADDR, 0);
        wr32(base, TDMA_RING_BASE + DMA_READ_PTR, 0);
        wr32(base, TDMA_RING_BASE + DMA_WRITE_PTR, 0);
        wr32(
            base,
            TDMA_RING_BASE + DMA_END_ADDR,
            (TOTAL_DESCS * DMA_DESC_SIZE / 4 - 1) as u32,
        );

        // Cannot init TDMA_PROD_INDEX to 0: align it on TDMA_CONS_INDEX.
        let cons = rd32(base, TDMA_RING_BASE + DMA_CONS_INDEX);
        wr32(base, TDMA_RING_BASE + DMA_PROD_INDEX, cons);
        self.tx_index = (cons & 0xFF) as usize;
        self.prod_index = cons;

        wr32(base, TDMA_RING_BASE + DMA_MBUF_DONE_THRESH, 1);
        wr32(base, TDMA_RING_BASE + DMA_FLOW_PERIOD, 0);
        wr32(
            base,
            TDMA_RING_BASE + DMA_RING_BUF_SIZE,
            ((TOTAL_DESCS << 16) | RX_BUF_LENGTH) as u32,
        );
        wr32(base, TDMA_CTRL_BASE + DMA_RING_CFG, 1 << DEFAULT_Q);
    }

    fn phy_init(&self) -> Result<u32, &'static str> {
        let base = self.base;
        let phy = self.phyaddr;

        // Soft reset, then wait for the self-clearing bit.
        mdio_write(base, phy, 0, 0x8000);
        let deadline = Instant::now() + Duration::from_secs(2);
        while mdio_read(base, phy, 0) & 0x8000 != 0 {
            if Instant::now() > deadline {
                return Err("phy reset timeout");
            }
            sleep_ms(10);
        }

        // Advertise 10/100 full+half, pause, and 1000BASE-T full.
        mdio_write(base, phy, 4, 0x0DE1);
        mdio_write(base, phy, 9, 0x0300);
        // Enable + restart autonegotiation.
        mdio_write(base, phy, 0, 0x1200);

        // Wait for link + autoneg complete (BMSR bits 2 and 5).
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let bmsr = mdio_read(base, phy, 1);
            if bmsr & 0x0024 == 0x0024 {
                break;
            }
            if Instant::now() > deadline {
                return Err("phy no link");
            }
            sleep_ms(100);
        }

        // Resolve speed (genphy_read_status logic).
        let lpa1000 = mdio_read(base, phy, 10);
        let lpa = mdio_read(base, phy, 5);
        let speed = if lpa1000 & 0x0C00 != 0 {
            1000
        } else if lpa & 0x0180 != 0 {
            100
        } else {
            10
        };
        Ok(speed)
    }

    fn adjust_link(&self, speed: u32) {
        let base = self.base;
        let v = rd32(base, EXT_RGMII_OOB_CTRL);
        wr32(
            base,
            EXT_RGMII_OOB_CTRL,
            (v & !OOB_DISABLE) | RGMII_LINK | RGMII_MODE_EN,
        );
        let v = rd32(base, EXT_RGMII_OOB_CTRL);
        wr32(base, EXT_RGMII_OOB_CTRL, v | ID_MODE_DIS);
        wr32(base, UMAC_CMD, speed_code(speed) << CMD_SPEED_SHIFT);
    }

    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// Build a 60-byte broadcast frame (EtherType 0x88B5) and hand it to the
    /// TX DMA ring. Blocks up to ~100 ms for the descriptor to be consumed.
    pub fn tx_beacon(&mut self, counter: u64, msg: &str) -> Result<(), &'static str> {
        let base = self.base;

        let mut frame = [0u8; 60];
        frame[0..6].copy_from_slice(&[0xFF; 6]);
        frame[6..12].copy_from_slice(&self.mac);
        frame[12] = 0x88;
        frame[13] = 0xB5;
        let payload = format!("AIOS-BOOT n={} {}", counter, msg);
        let n = payload.as_bytes().len().min(46);
        frame[14..14 + n].copy_from_slice(&payload.as_bytes()[..n]);

        let virt = self.tx_buf.virt_addr() as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), virt, frame.len());
        }
        let phys = self.tx_buf.physical();

        let desc = base + GENET_TX_OFF + self.tx_index * DMA_DESC_SIZE;
        wr32(desc, DMA_DESC_ADDRESS_LO, phys as u32);
        wr32(desc, DMA_DESC_ADDRESS_HI, (phys >> 32) as u32);
        let len_stat = ((frame.len() as u32) << DMA_BUFLENGTH_SHIFT)
            | (0x3F << DMA_TX_QTAG_SHIFT)
            | DMA_TX_APPEND_CRC
            | DMA_SOP
            | DMA_EOP;
        wr32(desc, DMA_DESC_LENGTH_STATUS, len_stat);

        self.tx_index = (self.tx_index + 1) % TOTAL_DESCS;
        self.prod_index = self.prod_index.wrapping_add(1);
        wr32(base, TDMA_RING_BASE + DMA_PROD_INDEX, self.prod_index);

        // Wait for the DMA consumer to catch up (U-Boot does the same).
        let target = self.prod_index & 0xFFFF;
        let mut tries = 100;
        loop {
            let cons = rd32(base, TDMA_RING_BASE + DMA_CONS_INDEX) & 0xFFFF;
            if cons == target {
                return Ok(());
            }
            tries -= 1;
            if tries == 0 {
                return Err("tx timeout");
            }
            sleep_ms(1);
        }
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
}

fn speed_code(speed: u32) -> u32 {
    match speed {
        1000 => UMAC_SPEED_1000,
        100 => UMAC_SPEED_100,
        _ => UMAC_SPEED_10,
    }
}

/// LED code for a negotiated speed: 1 = 10M, 2 = 100M, 3 = 1G.
pub fn speed_led_code(speed: u32) -> u32 {
    match speed {
        1000 => 3,
        100 => 2,
        _ => 1,
    }
}
