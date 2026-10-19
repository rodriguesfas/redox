//! Quanto tempo uma concessão (lease) vale e quando ela tem que ser renovada.
//!
//! RFC 2131 §4.4.5: o cliente começa a renovar em T1, pedindo a extensão
//! direto a quem concedeu (unicast), e, se ninguém responder, passa a
//! re-vincular em T2 (broadcast, qualquer servidor) até a concessão vencer.

use std::time::Duration;

/// Os tempos de uma concessão.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Timers {
    /// Até quando o endereço vale.
    pub lease: Duration,
    /// Quando começar a renovar com o servidor que concedeu.
    pub t1: Duration,
    /// Quando passar a re-vincular, se o servidor sumiu.
    pub t2: Duration,
}

/// Uma concessão viva: o endereço, quem concedeu e os tempos.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    /// Opção 54: o servidor que concedeu, para onde a renovação vai.
    pub server: Option<[u8; 4]>,
    /// Os tempos da concessão (opções 51, 58 e 59).
    pub timers: Timers,
    /// O endereço concedido.
    pub addr: [u8; 4],
}

/// O que fazer com a concessão depois de `elapsed` desde que ela foi aceita.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Ainda não é hora: esperar.
    Waiting,
    /// T1: renovar, unicast, com quem concedeu.
    Renew,
    /// T2: re-vincular, broadcast, até alguém atender.
    Rebind,
    /// Venceu: o endereço não é mais nosso; começar de novo.
    Expired,
}

impl Timers {
    /// Os tempos a partir das opções 51 (concessão), 58 (T1) e 59 (T2), em
    /// segundos.
    ///
    /// Sem a opção 51 não há concessão para manter: quem não sabe até quando
    /// pode usar o endereço não tem o que renovar. Sem T1/T2, vale o padrão do
    /// RFC — metade e sete oitavos da concessão. Tempos que não caibam na
    /// concessão (T1 ≥ T2, ou T2 ≥ concessão) caem no padrão também: renovar
    /// depois de o endereço vencer não é renovar.
    pub fn from_options(lease: Option<u32>, t1: Option<u32>, t2: Option<u32>) -> Option<Timers> {
        let lease = Duration::from_secs(lease.filter(|secs| *secs > 0)? as u64);
        let (default_t1, default_t2) = (lease / 2, lease * 7 / 8);

        let t1 = t1
            .filter(|secs| *secs > 0)
            .map(|secs| Duration::from_secs(secs as u64))
            .unwrap_or(default_t1);
        let t2 = t2
            .filter(|secs| *secs > 0)
            .map(|secs| Duration::from_secs(secs as u64))
            .unwrap_or(default_t2);

        let (t1, t2) = if t1 < t2 && t2 < lease {
            (t1, t2)
        } else {
            (default_t1, default_t2)
        };

        Some(Timers { lease, t1, t2 })
    }

    /// Os tempos depois de uma renovação.
    ///
    /// O ACK de uma renovação pode vir sem os tempos (a concessão continua a
    /// mesma), só com a concessão nova, ou com tudo. Sem nada, valem os tempos
    /// que já tínhamos; com a concessão, T1/T2 saem dela, do jeito que o
    /// servidor mandar.
    pub fn renewed(&self, lease: Option<u32>, t1: Option<u32>, t2: Option<u32>) -> Timers {
        if lease.is_none() && t1.is_none() && t2.is_none() {
            return *self;
        }

        Timers::from_options(lease.or(Some(self.lease.as_secs().max(1) as u32)), t1, t2)
            .unwrap_or(*self)
    }

    /// O estágio da concessão `elapsed` depois de ela ter sido aceita.
    pub fn at(&self, elapsed: Duration) -> Stage {
        if elapsed >= self.lease {
            Stage::Expired
        } else if elapsed >= self.t2 {
            Stage::Rebind
        } else if elapsed >= self.t1 {
            Stage::Renew
        } else {
            Stage::Waiting
        }
    }

    /// Quanto esperar, a partir de `elapsed`, para o próximo estágio.
    ///
    /// É o que deixa o laço dormir em vez de perguntar de segundo em segundo:
    /// em [`Stage::Waiting`] espera até T1, em [`Stage::Renew`] até T2, em
    /// [`Stage::Rebind`] até vencer, e em [`Stage::Expired`] não espera.
    pub fn wait(&self, elapsed: Duration) -> Duration {
        let until = match self.at(elapsed) {
            Stage::Waiting => self.t1,
            Stage::Renew => self.t2,
            Stage::Rebind => self.lease,
            Stage::Expired => return Duration::ZERO,
        };

        until.saturating_sub(elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn sem_concessao_nao_ha_o_que_renovar() {
        assert_eq!(Timers::from_options(None, None, None), None);
        assert_eq!(Timers::from_options(Some(0), None, None), None);
        assert_eq!(Timers::from_options(Some(0), Some(60), Some(120)), None);
    }

    #[test]
    fn sem_t1_t2_vale_metade_e_sete_oitavos() {
        let timers = Timers::from_options(Some(3600), None, None).unwrap();
        assert_eq!(timers.lease, secs(3600));
        assert_eq!(timers.t1, secs(1800));
        assert_eq!(timers.t2, secs(3150));
    }

    #[test]
    fn o_servidor_pode_dizer_t1_e_t2() {
        let timers = Timers::from_options(Some(600), Some(100), Some(200)).unwrap();
        assert_eq!(timers.lease, secs(600));
        assert_eq!(timers.t1, secs(100));
        assert_eq!(timers.t2, secs(200));
    }

    #[test]
    fn tempos_que_nao_cabem_na_concessao_caem_no_padrao() {
        // T2 depois de o endereço vencer: não haveria o que renovar.
        let timers = Timers::from_options(Some(600), Some(10), Some(601)).unwrap();
        assert_eq!((timers.t1, timers.t2), (secs(300), secs(525)));

        // T1 depois de T2.
        let timers = Timers::from_options(Some(600), Some(500), Some(100)).unwrap();
        assert_eq!((timers.t1, timers.t2), (secs(300), secs(525)));

        // Um zero é "não disse": o padrão.
        let timers = Timers::from_options(Some(600), Some(0), Some(0)).unwrap();
        assert_eq!((timers.t1, timers.t2), (secs(300), secs(525)));
    }

    #[test]
    fn os_estagios_seguem_t1_t2_e_o_vencimento() {
        let timers = Timers::from_options(Some(1000), Some(100), Some(900)).unwrap();

        assert_eq!(timers.at(secs(0)), Stage::Waiting);
        assert_eq!(timers.at(secs(99)), Stage::Waiting);
        assert_eq!(timers.at(secs(100)), Stage::Renew);
        assert_eq!(timers.at(secs(899)), Stage::Renew);
        assert_eq!(timers.at(secs(900)), Stage::Rebind);
        assert_eq!(timers.at(secs(999)), Stage::Rebind);
        assert_eq!(timers.at(secs(1000)), Stage::Expired);
        assert_eq!(timers.at(secs(10_000)), Stage::Expired);
    }

    #[test]
    fn a_espera_vai_ate_o_proximo_estagio() {
        let timers = Timers::from_options(Some(1000), Some(100), Some(900)).unwrap();

        assert_eq!(timers.wait(secs(0)), secs(100));
        assert_eq!(timers.wait(secs(99)), secs(1));
        assert_eq!(timers.wait(secs(100)), secs(800));
        assert_eq!(timers.wait(secs(900)), secs(100));
        assert_eq!(timers.wait(secs(1000)), Duration::ZERO);
    }

    #[test]
    fn uma_renovacao_sem_tempos_mantem_a_concessao() {
        let timers = Timers::from_options(Some(1000), Some(100), Some(900)).unwrap();

        assert_eq!(timers.renewed(None, None, None), timers);
    }

    #[test]
    fn uma_renovacao_com_concessao_nova_recalcula_t1_t2() {
        let timers = Timers::from_options(Some(1000), Some(100), Some(900)).unwrap();
        let renewed = timers.renewed(Some(600), None, None);

        assert_eq!(renewed.lease, secs(600));
        assert_eq!(renewed.t1, secs(300));
        assert_eq!(renewed.t2, secs(525));
    }

    #[test]
    fn uma_renovacao_so_com_t1_mantem_a_concessao() {
        let timers = Timers::from_options(Some(1000), Some(100), Some(900)).unwrap();
        let renewed = timers.renewed(None, Some(200), None);

        assert_eq!(renewed.lease, secs(1000));
        assert_eq!(renewed.t1, secs(200));
        assert_eq!(renewed.t2, secs(875));
    }
}
