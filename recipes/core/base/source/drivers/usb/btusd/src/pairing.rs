//! Bluetooth pairing state machine (SSP "JustWorks" + legado PIN).
//!
//! Dirigida por eventos HCI e por comandos do scheme ("pair <bdaddr>").
//! Politica padrao: aceita JustWorks automaticamente (io_cap NoInputNoOutput),
//! rejeita PIN quando nenhum foi configurado ("pin 1234" no ctl configura).

use alloc::string::String;
use alloc::vec::Vec;

use crate::hci::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairPhase {
    Idle,
    /// CreateConnection enviado; esperando Connection Complete.
    Connecting,
    /// Link iniciado; SSP/autenticacao em andamento.
    Authenticating,
    /// Pareado a nivel de link key, aguardando criptografia ligar.
    Encrypting,
    Paired,
    Failed,
}

/// Comando pendente gerado por um evento de pareamento.
pub struct EmittedCmd(pub HciCommand);

#[derive(Debug, Default)]
pub struct Pairing {
    phase: PairPhase,
    current: Option<[u8; 6]>,
    handle: Option<u16>,
    pin: Option<String>,
    link_keys: Vec<([u8; 6], [u8; 16])>,
    /// Politica: SSP auto-aceita (JustWorks). Desligar = falhar com PIN.
    pub auto_accept: bool,
}

impl Default for PairPhase {
    fn default() -> Self {
        Self::Idle
    }
}

impl Pairing {
    pub fn new() -> Self {
        Self {
            auto_accept: true,
            ..Default::default()
        }
    }

    pub fn phase(&self) -> PairPhase {
        self.phase
    }

    pub fn is_paired(&self, bd: [u8; 6]) -> bool {
        self.phase == PairPhase::Paired && self.current == Some(bd)
    }

    pub fn link_key(&self, bd: &[u8; 6]) -> Option<[u8; 16]> {
        self.link_keys
            .iter()
            .find(|(b, _)| b == bd)
            .map(|(_, k)| *k)
    }

    pub fn paired_device(&self) -> Option<[u8; 6]> {
        if self.phase == PairPhase::Paired {
            self.current
        } else {
            None
        }
    }

    /// Configura um PIN legado (quando o par pede PIN em vez de SSP).
    pub fn set_pin(&mut self, pin: String) {
        self.pin = Some(pin);
    }

    /// Inicia pareamento com um dispositivo descoberto. Emite CreateConnection.
    pub fn start_pair(&mut self, bd: [u8; 6]) -> HciCommand {
        self.current = Some(bd);
        self.phase = PairPhase::Connecting;
        cmd_create_connection(bd)
    }

    /// BD_ADDR texto "aa:bb:cc:dd:ee:ff" -> bytes.
    pub fn parse_bd_text(text: &str) -> Option<[u8; 6]> {
        let mut out = [0u8; 6];
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() != 6 {
            return None;
        }
        for (i, p) in parts.iter().enumerate() {
            out[i] = u8::from_str_radix(p.trim(), 16).ok()?;
        }
        Some(out)
    }

    /// Processa um evento HCI relevante ao pareamento.
    /// Retorna comandos HCI que o daemon deve enviar de volta.
    pub fn on_event(&mut self, code: u8, params: &[u8]) -> Vec<HciCommand> {
        let mut out: Vec<HciCommand> = Vec::new();
        match code {
            EVT_CONNECTION_COMPLETE => {
                let Some((status, handle, bd)) = parse_connection_complete(params) else {
                    return out;
                };
                if status != 0 {
                    self.phase = PairPhase::Failed;
                    return out;
                }
                self.current = Some(bd);
                self.handle = Some(handle);
                self.phase = PairPhase::Authenticating;
                // pede autenticacao (vai desencadear SSP ou PIN)
                out.push(cmd_auth_requested(handle));
            }
            EVT_CONNECTION_REQUEST => {
                // conexao de entrada: aceita se auto_accept
                if let Some(bd) = parse_connection_request(params) {
                    if self.auto_accept && self.phase == PairPhase::Idle {
                        self.current = Some(bd);
                        self.phase = PairPhase::Connecting;
                        out.push(cmd_accept_connection(bd, true));
                    } else {
                        out.push(HciCommand::with_params(CMD_REJECT_CONNECTION, {
                            let mut v = bd.to_vec();
                            v.push(0x0F); // Host Reject: Limited Resources
                            v
                        }));
                    }
                }
            }
            EVT_LINK_KEY_REQ => {
                if let Some(bd) = parse_bd_addr_event(params) {
                    match self.link_key(&bd) {
                        Some(key) => out.push(cmd_link_key_reply(bd, key)),
                        None => out.push(HciCommand::with_params(
                            CMD_LINK_KEY_NEGREPLY,
                            bd.to_vec(),
                        )),
                    }
                }
            }
            EVT_PIN_CODE_REQ => {
                if let Some(bd) = parse_bd_addr_event(params) {
                    match &self.pin {
                        Some(pin) => out.push(cmd_pin_reply(bd, pin)),
                        None => out.push(HciCommand::with_params(
                            CMD_PIN_NEGREPLY,
                            bd.to_vec(),
                        )),
                    }
                }
            }
            EVT_IO_CAP_REQ => {
                // JustWorks: NoInputNoOutput
                if let Some(bd) = parse_bd_addr_event(params) {
                    out.push(cmd_io_cap_reply(bd, IO_CAP_NO_INPUT_NO_OUTPUT, 0));
                }
            }
            EVT_USER_CONFIRM_REQ => {
                if let Some((bd, _num)) = parse_user_confirm_req(params) {
                    if self.auto_accept {
                        out.push(cmd_user_confirm_reply(bd));
                    } else {
                        out.push(HciCommand::with_params(
                            CMD_USER_CONFIRM_NEGREPLY,
                            bd.to_vec(),
                        ));
                    }
                }
            }
            EVT_SSP_COMPLETE => {
                if let Some((status, bd)) = parse_ssp_complete(params) {
                    if status == 0 {
                        // link key chegou via EVT_LINK_KEY_NOTIF; ligar criptografia
                        self.current = Some(bd);
                        if let Some(h) = self.handle {
                            self.phase = PairPhase::Encrypting;
                            out.push(cmd_set_conn_encrypt(h, true));
                        }
                    } else {
                        self.phase = PairPhase::Failed;
                    }
                }
            }
            EVT_LINK_KEY_NOTIF => {
                if let Some((bd, key, _ty)) = parse_link_key_notif(params) {
                    if self.link_key(&bd).is_none() {
                        self.link_keys.push((bd, key));
                    }
                }
            }
            EVT_AUTH_COMPLETE => {
                if let Some((status, _handle)) = parse_status_handle(params) {
                    if status != 0 {
                        self.phase = PairPhase::Failed;
                    }
                    // ok: segue o baile (encryption change fecha)
                }
            }
            EVT_ENCRYPTION_CHANGE => {
                if let Some((status, _handle)) = parse_status_handle(params) {
                    self.phase = if status == 0 {
                        PairPhase::Paired
                    } else {
                        PairPhase::Failed
                    };
                }
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const BD: [u8; 6] = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42];
    const H: u16 = 0x0020;
    const KEY: [u8; 16] = [0xAB; 16];

    fn conn_complete_ok() -> Vec<u8> {
        let mut p = vec![0x00];
        p.extend_from_slice(&H.to_le_bytes());
        p.extend_from_slice(&BD);
        p.extend_from_slice(&[0x01, 0x00]);
        p
    }

    /// Fluxo JustWorks totalmente feliz: connect -> auth req -> link key req
    /// (sem chave => negative -> SSP) -> io cap -> user confirm -> ssp ok ->
    /// link key notif -> encryption on -> Paired.
    #[test]
    fn justworks_fluxo_completo() {
        let mut pr = Pairing::new();
        let cmd = pr.start_pair(BD);
        assert_eq!(cmd.opcode, CMD_CREATE_CONNECTION);
        assert_eq!(cmd.params[..6], BD);

        // connection complete
        let evs = pr.on_event(EVT_CONNECTION_COMPLETE, &conn_complete_ok());
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].opcode, CMD_AUTH_REQUESTED);
        assert_eq!(pr.phase(), PairPhase::Authenticating);

        // controlador pergunta se ja temos link key: nao temos
        let evs = pr.on_event(EVT_LINK_KEY_REQ, &BD);
        assert_eq!(evs[0].opcode, CMD_LINK_KEY_NEGREPLY);

        // IO capability + user confirm (JustWorks => aceita tudo)
        let evs = pr.on_event(EVT_IO_CAP_REQ, &BD);
        assert_eq!(evs[0].opcode, CMD_IO_CAP_REPLY);
        assert_eq!(evs[0].params[6], IO_CAP_NO_INPUT_NO_OUTPUT);

        let mut uc = BD.to_vec();
        uc.extend_from_slice(&654321u32.to_le_bytes());
        let evs = pr.on_event(EVT_USER_CONFIRM_REQ, &uc);
        assert_eq!(evs[0].opcode, CMD_USER_CONFIRM_REPLY);

        // SSP concluiu
        let mut ssp = vec![0x00u8];
        ssp.extend_from_slice(&BD);
        let evs = pr.on_event(EVT_SSP_COMPLETE, &ssp);
        assert_eq!(pr.phase(), PairPhase::Encrypting);
        assert_eq!(evs[0].opcode, CMD_SET_CONN_ENCRYPT);
        assert_eq!(evs[0].params[..2], H.to_le_bytes());

        // link key notificada e guardada
        let mut kn = BD.to_vec();
        kn.extend_from_slice(&KEY);
        kn.push(4);
        pr.on_event(EVT_LINK_KEY_NOTIF, &kn);
        assert_eq!(pr.link_key(&BD), Some(KEY));

        // criptografia ligada => Paired
        pr.on_event(EVT_ENCRYPTION_CHANGE, &[0x00, 0x20, 0x00]);
        assert_eq!(pr.phase(), PairPhase::Paired);
        assert!(pr.is_paired(BD));
        assert_eq!(pr.paired_device(), Some(BD));
    }

    #[test]
    fn fluxo_legado_pin() {
        let mut pr = Pairing::new();
        pr.set_pin("1234".into());
        pr.start_pair(BD);
        pr.on_event(EVT_CONNECTION_COMPLETE, &conn_complete_ok());

        // pede PIN
        let evs = pr.on_event(EVT_PIN_CODE_REQ, &BD);
        assert_eq!(evs[0].opcode, CMD_PIN_REPLY);
        assert_eq!(&evs[0].params[7..11], b"1234");

        pr.on_event(EVT_AUTH_COMPLETE, &[0x00, 0x20, 0x00]);
        pr.on_event(EVT_ENCRYPTION_CHANGE, &[0x00, 0x20, 0x00]);
        assert_eq!(pr.phase(), PairPhase::Paired);
    }

    #[test]
    fn rejeicaobd() {
        // sem PIN configurado => PIN negative reply
        let mut pr = Pairing::new();
        let evs = pr.on_event(EVT_PIN_CODE_REQ, &BD);
        assert_eq!(evs[0].opcode, CMD_PIN_NEGREPLY);

        // ssp falha => Failed
        let fail = [0x06, 1, 2, 3, 4, 5, 6];
        let mut pr2 = Pairing::new();
        pr2.start_pair(BD);
        pr2.on_event(EVT_CONNECTION_COMPLETE, &conn_complete_ok());
        pr2.on_event(EVT_SSP_COMPLETE, &fail);
        assert_eq!(pr2.phase(), PairPhase::Failed);
    }

    #[test]
    fn inbound_aceito_e_rejeitado() {
        let mut pr = Pairing::new(); // auto_accept = true, Idle
        let mut req = BD.to_vec();
        req.extend_from_slice(&[0, 0, 0, 0x01]); // class + link type
        let evs = pr.on_event(EVT_CONNECTION_REQUEST, &req);
        assert_eq!(evs[0].opcode, CMD_ACCEPT_CONNECTION);

        // ocupado => rejeita
        let mut pr2 = Pairing::new();
        pr2.start_pair(BD);
        let evs = pr2.on_event(EVT_CONNECTION_REQUEST, &req);
        assert_eq!(evs[0].opcode, CMD_REJECT_CONNECTION);
        assert_eq!(evs[0].params[6], 0x0F);

        // auto_accept off => rejeita
        let mut pr3 = Pairing::new();
        pr3.auto_accept = false;
        let evs = pr3.on_event(EVT_CONNECTION_REQUEST, &req);
        assert_eq!(evs[0].opcode, CMD_REJECT_CONNECTION);
    }

    #[test]
    fn parse_bd_texto() {
        assert_eq!(
            Pairing::parse_bd_text("de:ad:be:ef:00:42").unwrap(),
            [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42]
        );
        assert!(Pairing::parse_bd_text("xx").is_none());
        assert!(Pairing::parse_bd_text("1:2:3:4:5").is_none());
        assert!(Pairing::parse_bd_text("01:02:03:04:05:zz").is_none());
    }
}
