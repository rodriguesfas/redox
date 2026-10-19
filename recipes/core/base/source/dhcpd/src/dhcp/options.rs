//! Leitura das opções que o cliente aproveita (RFC 2132).
//!
//! As opções chegam como uma sequência de `código, tamanho, dados` que termina
//! em 255 (ou no fim do buffer, se o servidor cortou). Um servidor que repita
//! uma opção não muda a primeira resposta.

/// O que o cliente aproveita de um OFFER ou de um ACK.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Opção 53: 1 DISCOVER, 2 OFFER, 3 REQUEST, 5 ACK, 6 NAK.
    pub message_type: Option<u8>,
    /// Opção 54: quem concedeu (o endereço para onde a renovação vai).
    pub server_id: Option<[u8; 4]>,
    /// Opção 1: a máscara da rede.
    pub subnet: Option<[u8; 4]>,
    /// Opção 3: o roteador padrão.
    pub router: Option<[u8; 4]>,
    /// Opção 6: o servidor de nomes.
    pub dns: Option<[u8; 4]>,
    /// Opção 51: quanto tempo a concessão vale, em segundos.
    pub lease: Option<u32>,
    /// Opção 58: quando começar a renovar, em segundos.
    pub t1: Option<u32>,
    /// Opção 59: quando re-vincular, em segundos.
    pub t2: Option<u32>,
}

/// Lê as opções de um OFFER ou ACK.
///
/// Uma opção cortada no fim (tamanho maior do que o que sobrou) encerra a
/// leitura: o que veio antes vale, o resto não existe.
pub fn parse(options: &[u8]) -> Options {
    let mut out = Options::default();
    let mut index = 0;

    while index < options.len() {
        let code = options[index];
        index += 1;

        match code {
            0 => continue, // pad
            255 => break,  // end
            _ => {}
        }

        if index >= options.len() {
            break;
        }
        let len = options[index] as usize;
        index += 1;
        if index + len > options.len() {
            break;
        }

        let data = &options[index..index + len];
        index += len;

        match code {
            1 => out.subnet = out.subnet.or_else(|| quad(data)),
            3 => out.router = out.router.or_else(|| quad(data)),
            6 => out.dns = out.dns.or_else(|| quad(data)),
            51 => out.lease = out.lease.or_else(|| seconds(data)),
            53 => out.message_type = out.message_type.or_else(|| one_byte(data)),
            54 => out.server_id = out.server_id.or_else(|| quad(data)),
            58 => out.t1 = out.t1.or_else(|| seconds(data)),
            59 => out.t2 = out.t2.or_else(|| seconds(data)),
            _ => {}
        }
    }

    out
}

fn quad(data: &[u8]) -> Option<[u8; 4]> {
    (data.len() == 4).then(|| [data[0], data[1], data[2], data[3]])
}

fn seconds(data: &[u8]) -> Option<u32> {
    quad(data).map(u32::from_be_bytes)
}

fn one_byte(data: &[u8]) -> Option<u8> {
    (data.len() == 1).then(|| data[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(entries: &[&[u8]]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|entry| entry.iter().copied())
            .collect()
    }

    #[test]
    fn le_o_offer_e_o_ack_do_servidor() {
        let options = buffer(&[
            &[1, 4, 255, 255, 255, 0], // máscara /24
            &[3, 4, 192, 168, 0, 1],   // roteador
            &[6, 4, 192, 168, 0, 53],  // DNS
            &[51, 4, 0, 0, 1, 44],     // 300s de concessão
            &[58, 4, 0, 0, 0, 100],    // T1
            &[59, 4, 0, 0, 0, 200],    // T2
            &[53, 1, 5],               // ACK
            &[54, 4, 192, 168, 0, 1],  // servidor
            &[255],
        ]);

        let parsed = parse(&options);

        assert_eq!(parsed.subnet, Some([255, 255, 255, 0]));
        assert_eq!(parsed.router, Some([192, 168, 0, 1]));
        assert_eq!(parsed.dns, Some([192, 168, 0, 53]));
        assert_eq!(parsed.lease, Some(300));
        assert_eq!(parsed.t1, Some(100));
        assert_eq!(parsed.t2, Some(200));
        assert_eq!(parsed.message_type, Some(5));
        assert_eq!(parsed.server_id, Some([192, 168, 0, 1]));
    }

    #[test]
    fn uma_opcao_cortada_no_fim_encerra_a_leitura() {
        // O que veio antes vale; a opção cortada (e o que viria depois) não.
        let options = buffer(&[
            &[3, 4, 10, 0, 0, 1], // roteador inteiro
            &[53, 1, 5],          // ACK
            &[51, 4, 0],          // concessão cortada: o buffer acaba antes
        ]);

        let parsed = parse(&options);

        assert_eq!(parsed.router, Some([10, 0, 0, 1]));
        assert_eq!(parsed.message_type, Some(5));
        assert_eq!(parsed.lease, None);
    }

    #[test]
    fn a_primeira_resposta_vale() {
        let options = buffer(&[&[51, 4, 0, 0, 0, 10], &[51, 4, 0, 0, 0, 99], &[255]]);

        assert_eq!(parse(&options).lease, Some(10));
    }

    #[test]
    fn opcoes_de_tamanho_errado_sao_ignoradas() {
        let options = buffer(&[
            &[1, 3, 255, 255, 0],    // máscara de três octetos
            &[53, 0],                // sem tipo
            &[58, 2, 0, 5],          // T1 de dois octetos
            &[54, 5, 1, 2, 3, 4, 5], // servidor de cinco
            &[255],
        ]);

        let parsed = parse(&options);

        assert_eq!(parsed.subnet, None);
        assert_eq!(parsed.message_type, None);
        assert_eq!(parsed.t1, None);
        assert_eq!(parsed.server_id, None);
    }

    #[test]
    fn o_pad_zero_nao_atrapalha_e_o_fim_encerra() {
        let options = buffer(&[&[0, 0], &[53, 1, 5], &[0], &[255], &[51, 4, 0, 0, 0, 7]]);

        let parsed = parse(&options);

        assert_eq!(parsed.message_type, Some(5));
        assert_eq!(
            parsed.lease, None,
            "o 255 encerra: o que vem depois não vale"
        );
    }

    #[test]
    fn sem_opcoes_nao_ha_nada() {
        assert_eq!(parse(&[]), Options::default());
        assert_eq!(parse(&[0, 0, 255]), Options::default());
    }
}
