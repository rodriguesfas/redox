use super::NvmeCmd;

impl NvmeCmd {
    pub fn create_io_completion_queue(
        cid: u16,
        qid: u16,
        ptr: usize,
        size: u16,
        iv: Option<u16>,
    ) -> Self {
        const DW11_PHYSICALLY_CONTIGUOUS_BIT: u32 = 0x0000_0001;
        const DW11_ENABLE_INTERRUPTS_BIT: u32 = 0x0000_0002;
        const DW11_INTERRUPT_VECTOR_SHIFT: u8 = 16;

        Self {
            opcode: 5,
            flags: 0,
            cid,
            nsid: 0,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr as u64, 0],
            cdw10: ((size as u32) << 16) | (qid as u32),

            cdw11: DW11_PHYSICALLY_CONTIGUOUS_BIT
                | if let Some(iv) = iv {
                    // enable interrupts if a vector is present
                    DW11_ENABLE_INTERRUPTS_BIT | (u32::from(iv) << DW11_INTERRUPT_VECTOR_SHIFT)
                } else {
                    0
                },

            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn create_io_submission_queue(
        cid: u16,
        qid: u16,
        ptr: usize,
        size: u16,
        cqid: u16,
    ) -> Self {
        Self {
            opcode: 1,
            flags: 0,
            cid,
            nsid: 0,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr as u64, 0],
            cdw10: ((size as u32) << 16) | (qid as u32),
            cdw11: ((cqid as u32) << 16) | 1, /* Physically Contiguous */
            //TODO: QPRIO
            cdw12: 0, //TODO: NVMSETID
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn identify_namespace(cid: u16, ptr: usize, nsid: u32) -> Self {
        Self {
            opcode: 6,
            flags: 0,
            cid,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr as u64, 0],
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn identify_controller(cid: u16, ptr: usize) -> Self {
        Self {
            opcode: 6,
            flags: 0,
            cid,
            nsid: 0,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr as u64, 0],
            cdw10: 1,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn identify_namespace_list(cid: u16, ptr: usize, base: u32) -> Self {
        Self {
            opcode: 6,
            flags: 0,
            cid,
            nsid: base,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr as u64, 0],
            cdw10: 2,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
    // TODO: this function is not used
    pub fn get_features(_cid: u16, ptr: usize, fid: u8) -> Self {
        Self {
            opcode: 0xA,
            dptr: [ptr as u64, 0],
            cdw10: u32::from(fid), // TODO: SEL
            ..Default::default()
        }
    }

    /// Dataset Management (DSM) — TRIM/Deallocate (opcode 0x09)
    ///
    /// dptr[0] aponta p/ uma tabela de DsmRange entries (16B cada).
    /// cdw10 = numero de ranges menos 1; cdw11 = atributos:
    ///   bit 2 (0x4): DEALLOCATE (osa os Freeform recovery behavior)
    pub fn dsm(
        cid: u16,
        nsid: u32,
        nranges: usize,
        attrs: DsmAttr,
        ptr_ranges: u64,
    ) -> Self {
        Self {
            opcode: 9,
            flags: 0,
            cid,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr_ranges, 0],
            cdw10: (nranges - 1) as u32,
            cdw11: attrs.bits(),
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn io_read(cid: u16, nsid: u32, lba: u64, blocks_1: u16, ptr0: u64, ptr1: u64) -> Self {
        Self {
            opcode: 2,
            flags: 0,
            cid,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr0, ptr1],
            cdw10: lba as u32,
            cdw11: (lba >> 32) as u32,
            cdw12: blocks_1 as u32,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    pub fn io_write(cid: u16, nsid: u32, lba: u64, blocks_1: u16, ptr0: u64, ptr1: u64) -> Self {
        Self {
            opcode: 1,
            flags: 0,
            cid,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr: [ptr0, ptr1],
            cdw10: lba as u32,
            cdw11: (lba >> 32) as u32,
            cdw12: blocks_1 as u32,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}


/// Atributos do comando Dataset Management (DSM)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsmAttr(u32);

impl DsmAttr {
    pub const NONE: Self = Self(0);
    pub const DEALLOCATE: Self = Self(0x4);
    pub const fn bits(self) -> u32 { self.0 }
}

/// Entrada de range DSM: 16 bytes por range.
/// Layout: [context_attrs:u32][length(logical blocks):u32][lba:u64]
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct DsmRange {
    pub ctx_attrs: u32,
    pub nlb: u32,
    pub lba: u64,
}

impl DsmRange {
    pub fn new(lba: u64, blocks: u32) -> Self {
        Self {
            ctx_attrs: 0,
            nlb: blocks,
            lba,
        }
    }

    /// Tamanho fixo da entrada = 16 bytes (usado p/ DMA)
    pub const SIZE: usize = 16;
}

#[cfg(test)]
mod dsm_tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn dsm_cmd_layout() {
        let cmd = NvmeCmd::dsm(
            7,
            1,
            16, // 16 ranges
            DsmAttr::DEALLOCATE,
            0xABCD_0000,
        );
        // NvmeCmd e packed: destruturar copia por valor (evita E0793)
        let NvmeCmd { opcode, cid, nsid, cdw10, cdw11, dptr, .. } = cmd;
        assert_eq!(opcode, 9);
        assert_eq!(nsid, 1);
        assert_eq!(cid, 7);
        assert_eq!(cdw10, 15);
        assert_eq!(cdw11, 0x4);
        assert_eq!(dptr[0], 0xABCD_0000u64);
    }

    #[test]
    fn dsm_range_layout() {
        assert_eq!(size_of::<DsmRange>(), DsmRange::SIZE);
        let r = DsmRange::new(0x1000, 8);
        let bytes: [u8; 16] = unsafe { core::mem::transmute(r) };
        // nlb=8 nos bytes 4..8
        assert_eq!(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), 8);
        // lba=0x1000 nos bytes 8..16
        assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 0x1000);
    }

    #[test]
    fn dsm_attr_bits() {
        assert_eq!(DsmAttr::NONE.bits(), 0);
        assert_eq!(DsmAttr::DEALLOCATE.bits(), 0x4);
    }
}
