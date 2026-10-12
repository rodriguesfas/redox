//! Probe state machine for USB Bluetooth HCI bring-up.
//!
//! Flow: HCI Reset -> Read Local Version -> Read BD_ADDR -> Ready.
//! Transport-agnostic: `poll()` yields the next command when none is pending;
//! `on_event()` consumes HCI events from the daemon's interrupt endpoint.

use crate::hci::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    New,
    ResetOk,
    VersionOk,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeReport {
    pub version: LocalVersion,
    pub bd_addr: [u8; 6],
}

#[derive(Debug)]
pub struct Probe {
    step: Step,
    /// Opcode do comando aguardando Command Complete.
    pending: Option<u16>,
    version: Option<LocalVersion>,
    bd_addr: Option<[u8; 6]>,
}

impl Probe {
    pub fn new() -> Self {
        Self {
            step: Step::New,
            pending: None,
            version: None,
            bd_addr: None,
        }
    }

    /// Comando a emitir agora (None quando esperando resposta ou pronto).
    pub fn poll(&mut self) -> Option<HciCommand> {
        if self.pending.is_some() {
            return None;
        }
        let cmd = match self.step {
            Step::New => HciCommand::new(CMD_RESET),
            Step::ResetOk => HciCommand::new(CMD_READ_LOCAL_VERSION),
            Step::VersionOk => HciCommand::new(CMD_READ_BD_ADDR),
            Step::Ready | Step::Failed => return None,
        };
        self.pending = Some(cmd.opcode);
        Some(cmd)
    }

    /// Consome um pacote de evento HCI. Erros sérios marcam Failed.
    pub fn on_event(&mut self, pkt: &[u8]) -> Result<(), &'static str> {
        let (code, params) = parse_event(pkt).ok_or("btusd: malformed HCI event")?;
        if code != EVT_COMMAND_COMPLETE {
            return Ok(()); // outros eventos ignorados no bring-up
        }
        let cc = parse_command_complete(params).ok_or("btusd: malformed CommandComplete")?;

        let pending = self.pending.ok_or("btusd: CommandComplete sem comando pendente")?;
        if cc.opcode != pending {
            self.step = Step::Failed;
            return Err("btusd: CommandComplete fora de ordem");
        }
        if cc.status != 0 {
            self.step = Step::Failed;
            return Err("btusd: HCI command falhou");
        }
        self.pending = None;

        match pending {
            CMD_RESET => self.step = Step::ResetOk,
            CMD_READ_LOCAL_VERSION => {
                self.version = Some(
                    LocalVersion::parse(&cc.return_params).ok_or("btusd: version payload ruim")?,
                );
                self.step = Step::VersionOk;
            }
            CMD_READ_BD_ADDR => {
                self.bd_addr = Some(parse_bd_addr(&cc.return_params).ok_or("btusd: bd addr ruim")?);
                self.step = Step::Ready;
            }
            _ => return Err("btusd: opcode inesperado"),
        }
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        self.step == Step::Ready
    }

    pub fn failed(&self) -> bool {
        self.step == Step::Failed
    }

    pub fn report(&self) -> Option<ProbeReport> {
        if !self.is_ready() {
            return None;
        }
        Some(ProbeReport {
            version: self.version?,
            bd_addr: self.bd_addr?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Monta um CommandComplete bem formado: [code 0x0E][len][1][opcode LE][status][ret...]
    fn cc(opcode: u16, status: u8, ret: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::new();
        pkt.push(0x0E);
        pkt.push((3 + 1 + ret.len()) as u8);
        pkt.push(1);
        pkt.extend_from_slice(&opcode.to_le_bytes());
        pkt.push(status);
        pkt.extend_from_slice(ret);
        pkt
    }

    const LOCAL_VER_RP: [u8; 8] = [0x0C, 0x00, 0x22, 0x01, 0x84, 0x08, 0x09, 0x00];
    const BD: [u8; 6] = [0x04, 0xCA, 0x38, 0x02, 0xAA, 0xBB];

    #[test]
    fn fluxo_completo_reset_versao_bdaddr() {
        let mut p = Probe::new();

        let c = p.poll().expect("reset");
        assert_eq!(c.opcode, CMD_RESET);
        assert!(p.poll().is_none());

        p.on_event(&cc(CMD_RESET, 0, &[])).unwrap();
        let c = p.poll().expect("read version");
        assert_eq!(c.opcode, CMD_READ_LOCAL_VERSION);
        assert!(!p.is_ready());

        p.on_event(&cc(CMD_READ_LOCAL_VERSION, 0, &LOCAL_VER_RP)).unwrap();
        let c = p.poll().expect("read bd addr");
        assert_eq!(c.opcode, CMD_READ_BD_ADDR);

        p.on_event(&cc(CMD_READ_BD_ADDR, 0, &BD)).unwrap();
        assert!(p.is_ready());
        assert!(p.poll().is_none());

        let r = p.report().unwrap();
        assert_eq!(r.bd_addr, BD);
        assert_eq!(r.version.hci_version_str(), "5.3");
        assert_eq!(r.version.manufacturer, 0x0884);
    }

    #[test]
    fn eventos_nao_cc_sao_ignorados() {
        let mut p = Probe::new();
        p.poll();
        // Inquiry Complete (0x01) no meio do reset pendente: ignora
        p.on_event(&[0x01, 0x01, 0x00]).unwrap();
        assert!(!p.failed());
    }

    #[test]
    fn status_nao_zero_falha() {
        let mut p = Probe::new();
        p.poll();
        assert!(p.on_event(&cc(CMD_RESET, 0x0C, &[])).is_err());
        assert!(p.failed());
        assert!(p.poll().is_none());
    }

    #[test]
    fn cc_sem_pendente_ou_fora_de_ordem_falha() {
        let mut p = Probe::new();
        // CC sem poll() antes
        assert!(p.on_event(&cc(CMD_RESET, 0, &[])).is_err());

        // fora de ordem
        p.poll(); // pending=RESET
        assert!(p.on_event(&cc(CMD_READ_LOCAL_VERSION, 0, &LOCAL_VER_RP)).is_err());
        assert!(p.failed());
    }
}
