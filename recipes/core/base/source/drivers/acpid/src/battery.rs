//! ACPI battery (_BST) parsing e consulta.
//!
//! `_BST` retorna um Package de 4 inteiros ([state_flags, mWh remanescente,
//! mWh da ultima carga completa, mV presente]). Fan questo
//! relatorio p/ userspace via o novo arquivo `battery<N>` no scheme acpid.

use amlserde::AmlSerdeValue;

/// Flags do campo state do _BST (ACPI spec 6.2.17)
pub const BST_DISCHARGING: u32 = 0x1;
pub const BST_CHARGING: u32 = 0x2;
pub const BST_CRITICAL: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    pub present: bool,
    pub charging: bool,
    pub critical: bool,
    /// energia restante (mWh ou mW dependendo do sistema power profile)
    pub remaining: u32,
    /// capacidade total da ultima carga completa
    pub capacity: u32,
    /// tensao presente (mV)
    pub voltage_mv: u32,
    /// percentual computado
    pub percent: u8,
}

impl BatteryStatus {
        pub fn to_line(&self) -> String {
        format!(
            "present={} charging={} critical={} remaining_mwh={} capacity_mwh={} percent={}\n",
            self.present as u8,
            self.charging as u8,
            self.critical as u8,
            self.remaining,
            self.capacity,
            self.percent,
        )
    }
}

/// Parsea um _BST (AmlSerdeValue::Package de 4 inteiros) em BatteryStatus.
pub fn parse_bst(value: &AmlSerdeValue) -> Result<BatteryStatus, &'static str> {
    let AmlSerdeValue::Package { contents } = value else {
        return Err("bst: nao e Package");
    };
    if contents.len() < 4 {
        return Err("bst: < 4 campos");
    }
    let vals: Result<Vec<u64>, &'static str> = contents[..4]
        .iter()
        .map(|v| match v {
            AmlSerdeValue::Integer(i) => Ok(*i),
            _ => Err("bst: campo nao-inteiro"),
        })
        .collect();
    let vals = vals?;

    let state = vals[0] as u32;
    let remaining = vals[1] as u32;
    let capacity = vals[2].max(1) as u32; // sem divisao por zero em hw drogado
    let voltage = vals[3] as u32;

    let percent = ((remaining as u64 * 100) / capacity as u64).min(100) as u8;

    Ok(BatteryStatus {
        present: true,
        charging: state & BST_CHARGING != 0,
        critical: state & BST_CRITICAL != 0,
        remaining,
        capacity,
        voltage_mv: voltage,
        percent,
    })
}

/// Encontra nomes de device de bateria num simbol-cache (chaves tipo
/// "_SB.PCI0.BAT0._BST"). Devolve os prefixos (sem o "._BST").
pub fn find_battery_names<'a>(symbols: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut out: Vec<String> = symbols
        .filter(|s| s.ends_with("._BST") || s.ends_with("._BIF"))
        .filter_map(|s| s.strip_suffix("._BST").or_else(|| s.strip_suffix("._BIF")))
        .map(|s| s.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    use std::string::String;

    fn pkg(vals: &[u64]) -> AmlSerdeValue {
        AmlSerdeValue::Package {
            contents: vals.iter().map(|&i| AmlSerdeValue::Integer(i)).collect(),
        }
    }

    #[test]
    fn bst_parse_carregando() {
        let s = parse_bst(&pkg(&[BST_CHARGING as u64, 40_000, 60_000, 12_500])).unwrap();
        assert!(s.present);
        assert!(s.charging);
        assert!(!s.critical);
        assert_eq!(s.percent, 66);
        assert!(s.to_line().contains("charging=1"));
    }

    #[test]
    fn bst_parse_descarregando() {
        let s = parse_bst(&pkg(&[BST_DISCHARGING as u64, 100, 10_000, 12_000])).unwrap();
        assert!(s.percent <= 1);
        assert!(!s.charging);
    }

    #[test]
    fn bst_rejeita_malformado() {
        assert!(parse_bst(&AmlSerdeValue::Integer(7)).is_err());
        assert!(parse_bst(&pkg(&[1, 2])).is_err()); // < 4 campos
    }

    #[test]
    fn find_batteries_dedup() {
        let names = vec![
            String::from("_SB.PCI0.BAT0._BST"),
            String::from("_SB.PCI0.BAT0._BIF"),
            String::from("_SB.PCI0.BAT1._BST"),
            String::from("_SB.PCI0.UAR1._PRW"), // nao-bateria
        ];
        let found = find_battery_names(names.iter());
        assert_eq!(found, vec![
            String::from("_SB.PCI0.BAT0"),
            String::from("_SB.PCI0.BAT1"),
        ]);
    }
}
