//! Volume/mute control para drivers HDA.
//!
//! Modelo: master em percentual (0-100) ->
//!  - gain byte do codec HDA (0..127, passos de ~0.75dB; 0x7F = 0dB, 0x00 = -95.25dB)
//!  - mute bit (1 = mudo)

/// Converte percentual 0..100 -> byte de gain (0..0x7F). Curva linear; silenci
/// absoluto so pode ser percebido via mute.
pub fn vol_to_gain(pct: u8) -> u8 {
    let pct = pct.min(100);
    ((pct as u32 * 0x7F) / 100) as u8
}

/// Percentual -> gain byte com rounding padrão (mais útil em UI).
pub fn vol_percent_to_codec(pct: u8) -> u8 {
    if pct >= 100 {
        return 0x7F;
    }
    if pct == 0 {
        return 0x00;
    }
    let g = ((pct as u32 * 0x7F) + 50) / 100;
    (g.min(0x7F)) as u8
}

/// Parsea "NN" ou "NN%" (aceita \n); erro se vazio ou >100.
pub fn parse_vol(buf: &[u8]) -> Result<u8, &'static str> {
    let s = core::str::from_utf8(buf).map_err(|_| "vol: utf8")?.trim();
    let s = s.trim_end_matches('%');
    if s.is_empty() {
        return Err("vol: vazio");
    }
    let v: u8 = s.parse().map_err(|_| "vol: nao-numero")?;
    if v > 100 {
        return Err("vol: fora de 0-100");
    }
    Ok(v)
}

/// Parsea "0"/"1"/"mute"/"on"
pub fn parse_mute(buf: &[u8]) -> Result<bool, &'static str> {
    let s = core::str::from_utf8(buf).map_err(|_| "mute: utf8")?.trim();
    match s {
        "" | "0" | "off" | "unmute" => Ok(false),
        "1" | "on" | "mute" | "mut" => Ok(true),
        _ => Err("mute: valor invalido"),
    }
}

/// DAC output path gain: aplica o mesmo ganho na sequencia DAC->sum->out
/// p/ tanto os canais quanto ambos os tempos.
/// Retorna (gain_byte, mute) consolidado.
pub fn resolve_master(pct: u8, muted: bool) -> (u8, bool) {
    (vol_percent_to_codec(pct), muted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_mapping() {
        assert_eq!(vol_to_gain(0), 0x00);
        assert_eq!(vol_to_gain(100), 0x7F);
        assert_eq!(vol_to_gain(50), 0x3F);
        assert_eq!(vol_to_gain(101), 0x7F); // saturado
    }

    #[test]
    fn percent_to_codec_rounded() {
        assert_eq!(vol_percent_to_codec(100), 0x7F);
        assert_eq!(vol_percent_to_codec(0), 0x00);
        assert_eq!(vol_percent_to_codec(50), 0x40); // arredonda para cima
        assert_eq!(vol_percent_to_codec(1), 1);
    }

    #[test]
    fn parse_volume_forms() {
        assert_eq!(parse_vol(b"80").unwrap(), 80);
        assert_eq!(parse_vol(b"80%").unwrap(), 80);
        assert_eq!(parse_vol(b"80\n").unwrap(), 80);
        assert_eq!(parse_vol(b"0").unwrap(), 0);
        assert!(parse_vol(b"101").is_err());
        assert!(parse_vol(b"-1").is_err());
        assert!(parse_vol(b"").is_err());
    }

    #[test]
    fn parse_mute_forms() {
        assert_eq!(parse_mute(b"1").unwrap(), true);
        assert_eq!(parse_mute(b"0").unwrap(), false);
        assert_eq!(parse_mute(b"mute\n").unwrap(), true);
        assert!(parse_mute(b"maybe").is_err());
    }

    #[test]
    fn resolve_master_consistente() {
        let (g, m) = resolve_master(75, false);
        assert_eq!(g, vol_percent_to_codec(75));
        assert!(!m);
        let (_, m) = resolve_master(75, true);
        assert!(m);
    }
}
