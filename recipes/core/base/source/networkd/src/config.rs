//! The network configuration: one file per interface, `*.network`.
//!
//! The format is the one `systemd-networkd` uses, cut down to what the network
//! stack here can actually do: a `[Match]` section naming the interface, an
//! `[Network]` section with the addressing, and one `[Route]` section per
//! route. The files live in a directory (`/etc/networkd`), are read in name
//! order, and the first one whose `[Match]` accepts an interface configures it
//! — so `10-eth0.network` beats `20-*.network`.
//!
//! Unknown sections and keys are reported and ignored: a configuration written
//! for a later version of the daemon has to leave the system working.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::addr::{Ipv4Addr, Ipv4Cidr, MacAddr, ParseError};

/// Where the configuration lives.
pub const CONFIG_DIR: &str = "/etc/networkd";

/// The extension of a network configuration file.
pub const FILE_EXT: &str = "network";

/// A `[Match]` section: which interfaces this file applies to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Match {
    /// Interface names, with `*` and `?` as wildcards.
    pub names: Vec<String>,
    /// Hardware addresses.
    pub macs: Vec<MacAddr>,
}

impl Match {
    /// Whether the interface is the one this file describes.
    ///
    /// An empty `[Match]` accepts nothing — a configuration that names no
    /// interface would otherwise apply to all of them, which is the opposite
    /// of what the file says.
    pub fn matches(&self, name: &str, mac: Option<MacAddr>) -> bool {
        let by_name = self
            .names
            .iter()
            .any(|pattern| wildcard_matches(pattern, name));
        let by_mac = mac.is_some_and(|mac| self.macs.contains(&mac));

        // Both are conditions, in the way a systemd unit lists them: a file
        // that names both is only for an interface that satisfies both.
        by_name || by_mac
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.macs.is_empty()
    }
}

/// A `[Route]` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The network to reach; `None` is the default route.
    pub destination: Option<Ipv4Cidr>,
    /// The address of the next hop.
    pub gateway: Ipv4Addr,
    /// The cost of the route, when the configuration gives one.
    pub metric: Option<u32>,
}

/// A `[Network]` section: the addressing of an interface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Network {
    /// A human readable name for the interface.
    pub description: Option<String>,
    /// Whether the address comes from a DHCP server.
    pub dhcp: Option<bool>,
    /// Static addresses, in the order they were written.
    pub addresses: Vec<Ipv4Cidr>,
    /// The address of the default route, when one is written.
    pub gateway: Option<Ipv4Addr>,
    /// The resolvers to use.
    pub dns: Vec<Ipv4Addr>,
}

impl Network {
    /// Whether the interface is left to a DHCP server.
    ///
    /// A file with no addressing at all says "configure this interface", and
    /// the only way to configure it without an address is DHCP.
    pub fn uses_dhcp(&self) -> bool {
        self.dhcp.unwrap_or(self.addresses.is_empty())
    }
}

/// One `*.network` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkFile {
    /// The file it was read from, when it came from one.
    pub source: Option<PathBuf>,
    /// The `[Match]` section.
    pub match_: Match,
    /// The `[Network]` section.
    pub network: Network,
    /// The `[Route]` sections.
    pub routes: Vec<Route>,
    /// What was understood but ignored, and what was not understood.
    pub warnings: Vec<String>,
}

impl NetworkFile {
    /// Reads one file, warning about the lines it does not understand.
    pub fn parse(text: &str, source: impl Into<Option<PathBuf>>) -> Result<Self, ConfigError> {
        let source = source.into();
        let name = source
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<configuration>".to_owned());

        let mut file = Self {
            source,
            ..Self::default()
        };
        let mut section = Section::None;
        // A `[Route]` is only added to the file once it has a next hop, so an
        // empty one does not become a route to nowhere.
        let mut route: Option<Route> = None;

        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.trim();

            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if let Some(title) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                if let Some(route) = route.take() {
                    file.add_route(route, &name, line_number)?;
                }

                section = match title.trim() {
                    "Match" => Section::Match,
                    "Network" => Section::Network,
                    "Route" => {
                        route = Some(Route {
                            destination: None,
                            gateway: Ipv4Addr::UNSPECIFIED,
                            metric: None,
                        });
                        Section::Route
                    }
                    other => {
                        file.warnings.push(format!(
                            "{name}:{line_number}: unknown section [{other}], ignored"
                        ));
                        Section::Unknown
                    }
                };
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                file.warnings.push(format!(
                    "{name}:{line_number}: not a `Key=Value` line, ignored"
                ));
                continue;
            };

            let key = key.trim();
            let value = value.trim();

            match section {
                Section::None => file.warnings.push(format!(
                    "{name}:{line_number}: {key}= before any section, ignored"
                )),
                Section::Unknown => (),
                Section::Match => file.set_match(key, value, &name, line_number)?,
                Section::Network => file.set_network(key, value, &name, line_number)?,
                Section::Route => {
                    let route = route
                        .as_mut()
                        .expect("a route section always starts a route");
                    set_route(route, key, value, &name, line_number, &mut file.warnings)?;
                }
            }
        }

        if let Some(route) = route.take() {
            file.add_route(route, &name, text.lines().count())?;
        }

        Ok(file)
    }

    /// Reads a directory of `*.network` files, in name order.
    ///
    /// A missing directory is not an error: a system with no configuration is
    /// a system where every interface is configured by DHCP, which is what the
    /// default boot does today.
    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, ConfigError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(ConfigError(format!(
                    "cannot read the configuration directory {}: {err}",
                    dir.display()
                )))
            }
        };

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|err| ConfigError(format!("cannot read {}: {err}", dir.display())))?;
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) != Some(FILE_EXT) {
                continue;
            }
            if !path.is_file() {
                continue;
            }

            paths.push(path);
        }
        paths.sort();

        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| ConfigError(format!("cannot read {}: {err}", path.display())))?;
            files.push(Self::parse(&text, Some(path))?);
        }

        Ok(files)
    }

    /// The interface this file describes, when it names exactly one.
    pub fn single_interface(&self) -> Option<&str> {
        match self.match_.names.as_slice() {
            [name] if !name.contains(['*', '?']) => Some(name),
            _ => None,
        }
    }

    fn add_route(
        &mut self,
        route: Route,
        name: &str,
        line_number: usize,
    ) -> Result<(), ConfigError> {
        if route.gateway.is_unspecified() {
            return Err(ConfigError(format!(
                "{name}:{line_number}: a [Route] needs a Gateway="
            )));
        }

        self.routes.push(route);
        Ok(())
    }

    fn set_match(
        &mut self,
        key: &str,
        value: &str,
        name: &str,
        line_number: usize,
    ) -> Result<(), ConfigError> {
        match key {
            "Name" => {
                for pattern in value.split_whitespace() {
                    self.match_.names.push(pattern.to_owned());
                }
            }
            "MACAddress" => {
                for part in value.split_whitespace() {
                    self.match_
                        .macs
                        .push(part.parse().map_err(|err: ParseError| {
                            ConfigError(format!("{name}:{line_number}: MACAddress: {err}"))
                        })?);
                }
            }
            other => self.warnings.push(format!(
                "{name}:{line_number}: unknown key {other}= in [Match], ignored"
            )),
        }

        Ok(())
    }

    fn set_network(
        &mut self,
        key: &str,
        value: &str,
        name: &str,
        line_number: usize,
    ) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError(format!("{name}:{line_number}: {message}"));

        match key {
            "Description" => self.network.description = Some(value.to_owned()),
            "DHCP" => {
                let dhcp = match value.to_ascii_lowercase().as_str() {
                    "yes" | "true" | "ipv4" | "on" | "1" => true,
                    "no" | "false" | "none" | "off" | "0" => false,
                    other => return Err(invalid(format!("DHCP={other}: the value is yes or no"))),
                };
                self.network.dhcp = Some(dhcp);
            }
            "Address" => {
                for part in value.split_whitespace() {
                    let cidr: Ipv4Cidr = part
                        .parse()
                        .map_err(|err: ParseError| invalid(format!("Address: {err}")))?;
                    if !cidr.addr().is_unicast() {
                        return Err(invalid(format!(
                            "Address={part}: not an address an interface can have"
                        )));
                    }
                    self.network.addresses.push(cidr);
                }
            }
            "Gateway" => {
                let gateway: Ipv4Addr = value
                    .parse()
                    .map_err(|err: ParseError| invalid(format!("Gateway: {err}")))?;
                if !gateway.is_unicast() {
                    return Err(invalid(format!(
                        "Gateway={value}: not an address a packet can be sent to"
                    )));
                }
                self.network.gateway = Some(gateway);
            }
            "DNS" => {
                for part in value.split_whitespace() {
                    let dns: Ipv4Addr = part
                        .parse()
                        .map_err(|err: ParseError| invalid(format!("DNS: {err}")))?;
                    if !dns.is_unicast() {
                        return Err(invalid(format!(
                            "DNS={part}: not an address a query can be sent to"
                        )));
                    }
                    self.network.dns.push(dns);
                }
            }
            other => self.warnings.push(format!(
                "{name}:{line_number}: unknown key {other}= in [Network], ignored"
            )),
        }

        Ok(())
    }
}

fn set_route(
    route: &mut Route,
    key: &str,
    value: &str,
    name: &str,
    line_number: usize,
    warnings: &mut Vec<String>,
) -> Result<(), ConfigError> {
    match key {
        "Destination" => {
            let destination = match value {
                "default" | "0.0.0.0/0" => None,
                other => {
                    let cidr: Ipv4Cidr = other.parse().map_err(|err: ParseError| {
                        ConfigError(format!("{name}:{line_number}: Destination: {err}"))
                    })?;
                    if !cidr.addr().is_unicast() {
                        return Err(ConfigError(format!(
                            "{name}:{line_number}: Destination={other}: not a network a packet can be sent to"
                        )));
                    }
                    Some(cidr.network())
                }
            };
            route.destination = destination;
        }
        "Gateway" => {
            let gateway: Ipv4Addr = value.parse().map_err(|err: ParseError| {
                ConfigError(format!("{name}:{line_number}: Gateway: {err}"))
            })?;
            if !gateway.is_unicast() {
                return Err(ConfigError(format!(
                    "{name}:{line_number}: Gateway={value}: not an address a packet can be sent to"
                )));
            }
            route.gateway = gateway;
        }
        "Metric" => {
            route.metric = Some(value.parse().map_err(|_| {
                ConfigError(format!("{name}:{line_number}: Metric={value}: a number"))
            })?);
        }
        other => warnings.push(format!(
            "{name}:{line_number}: unknown key {other}= in [Route], ignored"
        )),
    }

    Ok(())
}

/// Whether a pattern with `*` and `?` accepts a name.
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    fn matches(pattern: &[u8], name: &[u8]) -> bool {
        match pattern {
            [] => name.is_empty(),
            [b'*', rest @ ..] => {
                // `*` may take nothing, or one more character of the name.
                matches(rest, name) || (!name.is_empty() && matches(pattern, &name[1..]))
            }
            [b'?', rest @ ..] => !name.is_empty() && matches(rest, &name[1..]),
            [byte, rest @ ..] => name.first() == Some(byte) && matches(rest, &name[1..]),
        }
    }

    matches(pattern.as_bytes(), name.as_bytes())
}

/// A configuration that cannot be read or does not make sense.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// The configuration of a system: the files, in the order they are read.
#[derive(Clone, Debug, Default)]
pub struct Config {
    files: Vec<NetworkFile>,
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a directory of `*.network` files.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        Ok(Self {
            files: NetworkFile::load_dir(dir)?,
        })
    }

    /// The configuration written down, for a system that has no files.
    pub fn from_files(files: Vec<NetworkFile>) -> Self {
        Self { files }
    }

    pub fn files(&self) -> &[NetworkFile] {
        &self.files
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The file that configures an interface: the first one that matches.
    pub fn file_for(&self, name: &str, mac: Option<MacAddr>) -> Option<&NetworkFile> {
        self.files
            .iter()
            .find(|file| file.match_.matches(name, mac))
    }

    /// Everything that has to be reported about the configuration.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings: Vec<String> = Vec::new();

        for file in &self.files {
            let name = file
                .source
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<configuration>".to_owned());

            if file.match_.is_empty() {
                warnings.push(format!("{name}: no [Match], it configures nothing"));
            }

            warnings.extend(file.warnings.iter().cloned());
        }

        warnings
    }

    /// The configuration in the `.network` format, as it would be written.
    pub fn render(&self) -> String {
        let mut out = String::new();

        for file in &self.files {
            if !out.is_empty() {
                out.push('\n');
            }

            out.push_str("[Match]\n");
            if !file.match_.names.is_empty() {
                let _ = writeln!(out, "Name={}", file.match_.names.join(" "));
            }
            if !file.match_.macs.is_empty() {
                let macs: Vec<String> =
                    file.match_.macs.iter().map(|mac| mac.to_string()).collect();
                let _ = writeln!(out, "MACAddress={}", macs.join(" "));
            }

            out.push_str("\n[Network]\n");
            if let Some(description) = &file.network.description {
                let _ = writeln!(out, "Description={description}");
            }
            if let Some(dhcp) = file.network.dhcp {
                let _ = writeln!(out, "DHCP={}", if dhcp { "yes" } else { "no" });
            }
            for address in &file.network.addresses {
                let _ = writeln!(out, "Address={address}");
            }
            if let Some(gateway) = file.network.gateway {
                let _ = writeln!(out, "Gateway={gateway}");
            }
            for dns in &file.network.dns {
                let _ = writeln!(out, "DNS={dns}");
            }

            for route in &file.routes {
                out.push_str("\n[Route]\n");
                match route.destination {
                    Some(destination) => {
                        let _ = writeln!(out, "Destination={destination}");
                    }
                    None => out.push_str("Destination=default\n"),
                }
                let _ = writeln!(out, "Gateway={}", route.gateway);
                if let Some(metric) = route.metric {
                    let _ = writeln!(out, "Metric={metric}");
                }
            }
        }

        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    None,
    Match,
    Network,
    Route,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> NetworkFile {
        NetworkFile::parse(text, None).unwrap()
    }

    #[test]
    fn an_interface_is_described_by_its_match_section() {
        let file = parse(
            "[Match]\n\
             Name=eth0\n\
             \n\
             [Network]\n\
             Description=Ethernet\n\
             Address=192.168.1.50/24\n\
             Gateway=192.168.1.1\n\
             DNS=9.9.9.9 1.1.1.1\n",
        );

        assert_eq!(file.match_.names, vec!["eth0".to_owned()]);
        assert_eq!(file.network.description.as_deref(), Some("Ethernet"));
        assert_eq!(file.network.addresses.len(), 1);
        assert_eq!(file.network.addresses[0].to_string(), "192.168.1.50/24");
        assert_eq!(file.network.gateway.unwrap().to_string(), "192.168.1.1");
        assert_eq!(
            file.network
                .dns
                .iter()
                .map(|dns| dns.to_string())
                .collect::<Vec<_>>(),
            vec!["9.9.9.9", "1.1.1.1"]
        );

        // No DHCP= and an address: the address is what the file is for.
        assert!(!file.network.uses_dhcp());
        assert!(file.warnings.is_empty());
    }

    #[test]
    fn a_match_understands_wildcards_and_hardware_addresses() {
        let file = parse(
            "[Match]\n\
             Name=eth* enp?0\n\
             MACAddress=52:54:00:12:34:56\n",
        );

        assert_eq!(
            file.match_.names,
            vec!["eth*".to_owned(), "enp?0".to_owned()]
        );
        assert_eq!(file.match_.macs.len(), 1);
        assert!(file.match_.matches("eth0", None));
        assert!(file.match_.matches("enp10", None));
        assert!(!file.match_.matches("wlan0", None));
        assert!(file
            .match_
            .matches("wlan0", "52:54:00:12:34:56".parse().ok()));
        assert!(!file
            .match_
            .matches("wlan0", "52:54:00:12:34:57".parse().ok()));
    }

    #[test]
    fn a_wildcard_only_takes_what_it_has_to() {
        assert!(wildcard_matches("*", ""));
        assert!(wildcard_matches("*", "anything"));
        assert!(wildcard_matches("eth*", "eth0"));
        assert!(wildcard_matches("eth*", "eth"));
        assert!(wildcard_matches("*0", "eth0"));
        assert!(wildcard_matches("e*h0", "eth0"));
        assert!(wildcard_matches("??h0", "eth0"));
        assert!(!wildcard_matches("eth", "eth0"));
        assert!(!wildcard_matches("eth0", "eth"));
        assert!(wildcard_matches("?th0", "eth0"));
        assert!(wildcard_matches("eth?", "eth0"));
        assert!(!wildcard_matches("t?h0", "eth0"));
        assert!(!wildcard_matches("eth??", "eth0"));
        assert!(!wildcard_matches("*x*", "eth0"));
    }

    #[test]
    fn a_file_can_describe_dhcp() {
        let file = parse("[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n");
        assert!(file.network.uses_dhcp());

        let file = parse("[Match]\nName=eth0\n\n[Network]\nDHCP=no\nAddress=10.0.0.2/8\n");
        assert!(!file.network.uses_dhcp());
    }

    #[test]
    fn a_file_without_addressing_uses_dhcp() {
        // Saying nothing about the address is a way of saying DHCP: there is
        // no other way to give an interface an address.
        let file = parse("[Match]\nName=eth0\n\n[Network]\nDNS=9.9.9.9\n");
        assert!(file.network.uses_dhcp());
    }

    #[test]
    fn routes_are_read_one_section_at_a_time() {
        let file = parse(
            "[Match]\n\
             Name=eth0\n\
             \n\
             [Network]\n\
             Address=10.0.0.2/24\n\
             \n\
             [Route]\n\
             Destination=192.168.0.0/16\n\
             Gateway=10.0.0.1\n\
             Metric=100\n\
             \n\
             [Route]\n\
             Destination=default\n\
             Gateway=10.0.0.254\n",
        );

        assert_eq!(file.routes.len(), 2);
        assert_eq!(
            file.routes[0].destination.unwrap().to_string(),
            "192.168.0.0/16"
        );
        assert_eq!(file.routes[0].gateway.to_string(), "10.0.0.1");
        assert_eq!(file.routes[0].metric, Some(100));
        assert_eq!(file.routes[1].destination, None);
        assert_eq!(file.routes[1].gateway.to_string(), "10.0.0.254");
    }

    #[test]
    fn a_route_without_a_next_hop_is_refused() {
        let error = NetworkFile::parse(
            "[Match]\nName=eth0\n\n[Route]\nDestination=10.0.0.0/8\n",
            None,
        )
        .unwrap_err();
        assert!(error.0.contains("needs a Gateway="), "{error}");
    }

    #[test]
    fn what_is_not_understood_is_reported_and_ignored() {
        let file = parse(
            "[Match]\n\
             Name=eth0\n\
             Address=10.0.0.2/8\n\
             \n\
             [Wireless]\n\
             SSID=home\n\
             \n\
             [Network]\n\
             IPv6AcceptRA=yes\n\
             Address=10.0.0.3/8\n\
             not a key value line\n",
        );

        assert_eq!(file.network.addresses.len(), 1);
        assert_eq!(file.warnings.len(), 4, "{:?}", file.warnings);
        assert!(
            file.warnings[0].contains("unknown key Address= in [Match]"),
            "{:?}",
            file.warnings
        );
        assert!(file.warnings[1].contains("unknown section [Wireless]"));
        assert!(file.warnings[2].contains("unknown key IPv6AcceptRA= in [Network]"));
        assert!(file.warnings[3].contains("not a `Key=Value` line"));
    }

    #[test]
    fn a_value_that_is_not_a_value_is_an_error() {
        for (text, message) in [
            (
                "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.300/8\n",
                "Address",
            ),
            (
                "[Match]\nName=eth0\n\n[Network]\nAddress=0.0.0.0/8\n",
                "not an address an interface can have",
            ),
            (
                "[Match]\nName=eth0\n\n[Network]\nGateway=224.0.0.1\n",
                "not an address a packet can be sent to",
            ),
            (
                "[Match]\nName=eth0\n\n[Network]\nDNS=255.255.255.255\n",
                "not an address a query can be sent to",
            ),
            (
                "[Match]\nName=eth0\n\n[Network]\nDHCP=maybe\n",
                "DHCP=maybe",
            ),
            ("[Match]\nMACAddress=01:23:45:67:89\n", "MACAddress"),
            (
                "[Match]\nName=eth0\n\n[Route]\nGateway=10.0.0.1\nMetric=fast\n",
                "Metric=fast",
            ),
        ] {
            let error = NetworkFile::parse(text, None).unwrap_err();
            assert!(error.0.contains(message), "{text:?}: {error}");
        }
    }

    #[test]
    fn the_first_file_that_matches_configures_the_interface() {
        let config = Config::from_files(vec![
            parse("[Match]\nName=eth*\n\n[Network]\nDHCP=yes\n"),
            parse("[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n"),
        ]);

        let file = config.file_for("eth0", None).unwrap();
        assert_eq!(file.match_.names, vec!["eth*".to_owned()]);
        assert!(config.file_for("wlan0", None).is_none());
    }

    #[test]
    fn a_configuration_is_written_back_the_way_it_was_read() {
        let text = "[Match]\n\
                    Name=eth0\n\
                    \n\
                    [Network]\n\
                    Description=Ethernet\n\
                    DHCP=no\n\
                    Address=192.168.1.50/24\n\
                    Gateway=192.168.1.1\n\
                    DNS=9.9.9.9\n\
                    \n\
                    [Route]\n\
                    Destination=192.168.0.0/16\n\
                    Gateway=192.168.1.1\n\
                    Metric=100\n";
        let file = parse(text);
        let rendered = Config::from_files(vec![file.clone()]).render();

        assert_eq!(rendered, text);

        // And what was rendered reads back as the same file.
        let again = parse(&rendered);
        assert_eq!(again.network, file.network);
        assert_eq!(again.match_, file.match_);
        assert_eq!(again.routes, file.routes);
    }

    #[test]
    fn a_default_route_is_written_as_default() {
        let file = parse(
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n\n[Route]\nDestination=default\nGateway=10.0.0.1\n",
        );
        let rendered = Config::from_files(vec![file]).render();
        assert!(rendered.contains("Destination=default\n"), "{rendered}");
    }

    #[test]
    fn a_file_that_matches_nothing_is_reported() {
        let config = Config::from_files(vec![parse("[Network]\nDHCP=yes\n")]);
        let warnings = config.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no [Match]"), "{warnings:?}");
    }
}
