//! The addresses the network configuration is written with.
//!
//! The types are small on purpose: the configuration has to be validated long
//! before it reaches the network stack, and an address that the stack would
//! reject has to be rejected here, with a message that says so.

use std::fmt;
use std::str::FromStr;

/// An address or a value in the configuration that is not understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// An IPv4 address, in the order it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Ipv4Addr([u8; 4]);

impl Ipv4Addr {
    /// `0.0.0.0`: an address that names no interface.
    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);

    pub const fn new(octets: [u8; 4]) -> Self {
        Self(octets)
    }

    pub const fn octets(&self) -> [u8; 4] {
        self.0
    }

    pub const fn to_bits(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits.to_be_bytes())
    }

    pub const fn is_unspecified(&self) -> bool {
        self.to_bits() == 0
    }

    pub const fn is_broadcast(&self) -> bool {
        self.to_bits() == u32::MAX
    }

    /// `224.0.0.0/4`.
    pub const fn is_multicast(&self) -> bool {
        self.0[0] & 0xf0 == 0xe0
    }

    /// An address that can be given to an interface or be a resolver.
    ///
    /// This is the test the network stack itself applies: the unspecified
    /// address, the broadcast address and the multicast range are refused.
    pub const fn is_unicast(&self) -> bool {
        !self.is_unspecified() && !self.is_broadcast() && !self.is_multicast()
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

impl FromStr for Ipv4Addr {
    type Err = ParseError;

    /// Parses the dotted quad, with each octet as a decimal number.
    ///
    /// Nothing else is accepted: no shorthand (`10.1`), no hex, no leading
    /// sign — an address that a shell would expand differently is a bug
    /// waiting to happen.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let mut octets = [0u8; 4];
        let mut count = 0;

        for part in text.split('.') {
            if count == 4 {
                return Err(ParseError::new(format!("{text}: too many octets")));
            }

            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError::new(format!("{text}: not an IPv4 address")));
            }

            // More than three digits cannot fit an octet, and a leading zero
            // would read as octal elsewhere.
            if part.len() > 3 || (part.len() > 1 && part.starts_with('0')) {
                return Err(ParseError::new(format!("{text}: not an IPv4 address")));
            }

            octets[count] = part
                .parse()
                .map_err(|_| ParseError::new(format!("{text}: octet out of range")))?;
            count += 1;
        }

        if count != 4 {
            return Err(ParseError::new(format!(
                "{text}: an IPv4 address has four octets"
            )));
        }

        Ok(Self(octets))
    }
}

/// An IPv4 address with the length of its network prefix.
///
/// The configuration and the network stack both talk about addresses this way
/// (`192.168.1.50/24`), so the split between the address and the network it
/// belongs to is kept where it cannot be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Cidr {
    addr: Ipv4Addr,
    prefix: u8,
}

impl Ipv4Cidr {
    pub fn new(addr: Ipv4Addr, prefix: u8) -> Result<Self, ParseError> {
        if prefix > 32 {
            return Err(ParseError::new(format!(
                "{addr}/{prefix}: the prefix of an IPv4 network is at most 32"
            )));
        }

        Ok(Self { addr, prefix })
    }

    pub const fn addr(&self) -> Ipv4Addr {
        self.addr
    }

    pub const fn prefix(&self) -> u8 {
        self.prefix
    }

    /// The network mask, as an address.
    pub const fn mask(&self) -> Ipv4Addr {
        let bits = if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix)
        };
        Ipv4Addr::from_bits(bits)
    }

    /// The network this address belongs to, with the address set to its first.
    pub fn network(&self) -> Self {
        Self {
            addr: Ipv4Addr::from_bits(self.addr.to_bits() & self.mask().to_bits()),
            prefix: self.prefix,
        }
    }

    pub fn contains(&self, addr: Ipv4Addr) -> bool {
        addr.to_bits() & self.mask().to_bits() == self.network().addr().to_bits()
    }
}

impl fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl FromStr for Ipv4Cidr {
    type Err = ParseError;

    /// Parses `A.B.C.D/len`, or `A.B.C.D` for a single host (`/32`).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (text, None),
        };

        let addr = addr.parse()?;
        let prefix = match prefix {
            Some(prefix) if !prefix.is_empty() => prefix.parse().map_err(|_| {
                ParseError::new(format!("{text}: the prefix is a number between 0 and 32"))
            })?,
            Some(_) => {
                return Err(ParseError::new(format!(
                    "{text}: the prefix is a number between 0 and 32"
                )))
            }
            None => 32,
        };

        Self::new(addr, prefix)
    }
}

/// A hardware address, as the network stack writes it (`aa:bb:cc:dd:ee:ff`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    pub const fn octets(&self) -> [u8; 6] {
        self.0
    }

    /// Whether the address is a group address.
    ///
    /// The network stack refuses a group address as the address of an
    /// interface, so a configuration that does the same is refused here.
    pub const fn is_multicast(&self) -> bool {
        self.0[0] & 1 == 1
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

impl FromStr for MacAddr {
    type Err = ParseError;

    /// Parses six pairs of hexadecimal digits, separated by `:` or `-`.
    ///
    /// The two separators are the two ways the address is written in the
    /// system, and a mixed spelling is a typo, not an address.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let separator = if text.contains(':') {
            ':'
        } else if text.contains('-') {
            '-'
        } else {
            return Err(ParseError::new(format!(
                "{text}: a hardware address is written as aa:bb:cc:dd:ee:ff"
            )));
        };

        let mut octets = [0u8; 6];
        let mut count = 0;

        for part in text.split(separator) {
            if count == 6 || part.len() != 2 {
                return Err(ParseError::new(format!(
                    "{text}: a hardware address is six pairs of hex digits"
                )));
            }

            octets[count] = u8::from_str_radix(part, 16).map_err(|_| {
                ParseError::new(format!(
                    "{text}: a hardware address is six pairs of hex digits"
                ))
            })?;
            count += 1;
        }

        if count != 6 {
            return Err(ParseError::new(format!(
                "{text}: a hardware address is six pairs of hex digits"
            )));
        }

        Ok(Self(octets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_four_decimal_octets() {
        let addr: Ipv4Addr = "192.168.1.50".parse().unwrap();
        assert_eq!(addr.octets(), [192, 168, 1, 50]);
        assert_eq!(addr.to_string(), "192.168.1.50");
        assert_eq!(addr.to_bits(), 0xc0a8_0132);
        assert!(addr.is_unicast());

        assert_eq!(
            "0.0.0.0".parse::<Ipv4Addr>().unwrap(),
            Ipv4Addr::UNSPECIFIED
        );
        assert_eq!(
            "255.255.255.255".parse::<Ipv4Addr>().unwrap().octets(),
            [255; 4]
        );
    }

    #[test]
    fn an_address_that_is_not_a_dotted_quad_is_refused() {
        for text in [
            "",
            ".",
            "1.2.3",
            "1.2.3.4.5",
            "1.2.3.",
            "1.2.3.256",
            "256.1.1.1",
            "10.1",
            "0x10.1.1.1",
            "1.2.3.04",
            "010.1.1.1",
            "1.2.3.-1",
            "a.b.c.d",
        ] {
            assert!(
                text.trim().parse::<Ipv4Addr>().is_err(),
                "{text:?} should not parse"
            );
        }

        // Surrounding space is not part of the address.
        assert_eq!(
            " 10.0.0.1 ".parse::<Ipv4Addr>().unwrap().to_string(),
            "10.0.0.1"
        );
        assert!("0".parse::<Ipv4Addr>().is_err());
    }

    #[test]
    fn the_kinds_of_address_the_stack_refuses_are_known() {
        assert!("0.0.0.0".parse::<Ipv4Addr>().unwrap().is_unspecified());
        assert!("255.255.255.255"
            .parse::<Ipv4Addr>()
            .unwrap()
            .is_broadcast());
        assert!("224.0.0.1".parse::<Ipv4Addr>().unwrap().is_multicast());
        assert!("239.255.255.255"
            .parse::<Ipv4Addr>()
            .unwrap()
            .is_multicast());

        for text in ["0.0.0.0", "255.255.255.255", "224.0.0.1"] {
            assert!(
                !text.parse::<Ipv4Addr>().unwrap().is_unicast(),
                "{text} is not a unicast address"
            );
        }

        // 223.255.255.255 is the last unicast address, 240.0.0.0 is reserved
        // but not multicast, and a class A address is plain unicast.
        for text in ["223.255.255.255", "10.0.0.1", "192.168.1.1"] {
            assert!(text.parse::<Ipv4Addr>().unwrap().is_unicast(), "{text}");
        }
    }

    #[test]
    fn a_cidr_knows_its_mask_and_its_network() {
        let cidr: Ipv4Cidr = "192.168.1.50/24".parse().unwrap();
        assert_eq!(cidr.addr().to_string(), "192.168.1.50");
        assert_eq!(cidr.prefix(), 24);
        assert_eq!(cidr.mask().to_string(), "255.255.255.0");
        assert_eq!(cidr.network().to_string(), "192.168.1.0/24");
        assert!(cidr.contains("192.168.1.1".parse().unwrap()));
        assert!(!cidr.contains("192.168.2.1".parse().unwrap()));

        // A single host is the default, and the extremes are allowed.
        assert_eq!(
            "10.0.0.1".parse::<Ipv4Cidr>().unwrap().to_string(),
            "10.0.0.1/32"
        );
        assert_eq!(
            "0.0.0.0/0".parse::<Ipv4Cidr>().unwrap().mask().to_string(),
            "0.0.0.0"
        );
        assert_eq!(
            "0.0.0.0/0"
                .parse::<Ipv4Cidr>()
                .unwrap()
                .network()
                .to_string(),
            "0.0.0.0/0"
        );
        assert_eq!(
            "10.0.0.1/32"
                .parse::<Ipv4Cidr>()
                .unwrap()
                .mask()
                .to_string(),
            "255.255.255.255"
        );
        assert_eq!(
            "10.0.0.1/31"
                .parse::<Ipv4Cidr>()
                .unwrap()
                .mask()
                .to_string(),
            "255.255.255.254"
        );
    }

    #[test]
    fn a_cidr_that_is_not_one_is_refused() {
        for text in [
            "10.0.0.1/33",
            "10.0.0.1/",
            "10.0.0.1/x",
            "/24",
            "10.0.0.1/-1",
            "10.0.0.1/24/24",
        ] {
            assert!(
                text.parse::<Ipv4Cidr>().is_err(),
                "{text:?} should not parse"
            );
        }
    }

    #[test]
    fn a_hardware_address_is_six_pairs_of_hex_digits() {
        let mac: MacAddr = "52:54:00:12:34:56".parse().unwrap();
        assert_eq!(mac.octets(), [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        assert_eq!(mac.to_string(), "52:54:00:12:34:56");
        assert!(!mac.is_multicast());

        // The other spelling of the system, and the case of the digits.
        assert_eq!("52-54-00-12-34-56".parse::<MacAddr>().unwrap(), mac);
        assert_eq!(
            "AA:BB:CC:DD:EE:F0".parse::<MacAddr>().unwrap().to_string(),
            "aa:bb:cc:dd:ee:f0"
        );

        // The low bit of the first octet is what makes an address a group
        // address, which the stack refuses to put on an interface.
        for text in [
            "01:00:5e:00:00:01",
            "33:33:00:00:00:01",
            "01:23:45:67:89:ab",
        ] {
            assert!(text.parse::<MacAddr>().unwrap().is_multicast(), "{text}");
        }
    }

    #[test]
    fn a_hardware_address_that_is_not_one_is_refused() {
        for text in [
            "",
            "01:23:45:67:89",
            "01:23:45:67:89:ab:cd",
            "1:23:45:67:89:ab",
            "01:23:45:67:89:ag",
            "01.23.45.67.89.ab",
            "01:23-45:67:89:ab",
            "01234:23:45:67:89:ab",
        ] {
            assert!(
                text.parse::<MacAddr>().is_err(),
                "{text:?} should not parse"
            );
        }
    }
}
