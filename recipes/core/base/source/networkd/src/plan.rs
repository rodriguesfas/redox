//! What the configuration means for the interfaces of the system.
//!
//! A [`Plan`] is the configuration resolved against the interfaces that are
//! actually there: which interface is left to DHCP, which one gets a static
//! address, which routes and which resolvers. It is then turned into the exact
//! writes the network stack understands — the contract of `/scheme/netcfg`,
//! spelled out in one place instead of being repeated by every caller.
//!
//! Nothing here touches the system: a plan is built from the configuration and
//! an observation of the interfaces, so it can be built and checked on the
//! host.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::addr::{Ipv4Addr, Ipv4Cidr, MacAddr};
use crate::config::{Config, Route};

/// An interface as the system reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    /// The name of the interface (`eth0`).
    pub name: String,
    /// Its hardware address, when it has one.
    pub mac: Option<MacAddr>,
}

/// How an interface gets its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The address comes from a DHCP server.
    Dhcp,
    /// The address is written in the configuration.
    Static,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dhcp => "dhcp",
            Self::Static => "static",
        }
    }
}

/// What the configuration says about one interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfacePlan {
    /// The name of the interface.
    pub name: String,
    /// Its hardware address, when the system reported one.
    pub mac: Option<MacAddr>,
    /// The description from the configuration.
    pub description: Option<String>,
    /// How the address is decided.
    pub mode: Mode,
    /// The static addresses.
    pub addresses: Vec<Ipv4Cidr>,
    /// The next hop of the default route.
    pub gateway: Option<Ipv4Addr>,
    /// The routes written for this interface.
    pub routes: Vec<Route>,
    /// The resolvers written for this interface.
    pub dns: Vec<Ipv4Addr>,
    /// Whether a file of the configuration describes this interface.
    pub configured: bool,
    /// The file this interface was configured from, when it is known.
    pub source: Option<PathBuf>,
}

impl InterfacePlan {
    /// Whether the interface is configured by a file of its own.
    pub fn is_configured(&self) -> bool {
        self.configured
    }
}

/// A write to the network stack: one control file, one value.
///
/// The stack reads its control files as `write a value, then read the file` —
/// writing `/scheme/netcfg/ifaces/eth0/addr/set` is what gives `eth0` an
/// address. Keeping the writes as values means the same plan can be printed,
/// checked, or applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    /// The control file, below the root of the scheme.
    pub path: String,
    /// What to write to it, exactly as the stack expects it.
    pub value: String,
}

impl Write {
    /// The control file with the root of the scheme in front.
    pub fn full_path(&self, root: &str) -> String {
        let root = root.trim_end_matches('/');
        format!("{root}/{}", self.path)
    }

    /// The write, as something a person can read.
    pub fn render(&self, root: &str) -> String {
        format!("write {} {}", self.full_path(root), self.value.trim_end())
    }
}

/// What the configuration means for the whole system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// One entry per interface, in the order they were observed.
    pub interfaces: Vec<InterfacePlan>,
    /// What the configuration and the observations have to say.
    pub warnings: Vec<String>,
}

impl Plan {
    /// Resolves the configuration against the interfaces of the system.
    ///
    /// An interface no file matches is left to DHCP — a system with no
    /// configuration, or an interface plugged in later, still comes up. The
    /// warning says so, because "no file matched" and "a file said DHCP" are
    /// different situations for whoever is reading the log.
    pub fn build(config: &Config, observed: &[Observed]) -> Self {
        let mut warnings = config.warnings();
        let mut interfaces = Vec::with_capacity(observed.len());

        for iface in observed {
            let Some(file) = config.file_for(&iface.name, iface.mac) else {
                warnings.push(format!(
                    "{}: no configuration matches, using DHCP",
                    iface.name
                ));
                interfaces.push(InterfacePlan {
                    name: iface.name.clone(),
                    mac: iface.mac,
                    description: None,
                    mode: Mode::Dhcp,
                    addresses: Vec::new(),
                    gateway: None,
                    routes: Vec::new(),
                    dns: Vec::new(),
                    configured: false,
                    source: None,
                });
                continue;
            };

            let mode = if file.network.uses_dhcp() {
                Mode::Dhcp
            } else {
                Mode::Static
            };

            interfaces.push(InterfacePlan {
                name: iface.name.clone(),
                mac: iface.mac,
                description: file.network.description.clone(),
                mode,
                addresses: file.network.addresses.clone(),
                gateway: file.network.gateway,
                routes: file.routes.clone(),
                dns: file.network.dns.clone(),
                configured: true,
                source: file.source.clone(),
            });
        }

        Self {
            interfaces,
            warnings,
        }
    }

    /// The writes that make the system look like the plan.
    ///
    /// A DHCP interface is not addressed here: its address arrives from the
    /// server, and the route and the resolver arrive with it. A resolver
    /// written in the configuration is still set, because that is a decision
    /// the configuration made on purpose.
    pub fn writes(&self) -> Vec<Write> {
        let mut writes = Vec::new();

        for iface in &self.interfaces {
            if iface.mode == Mode::Static {
                for address in &iface.addresses {
                    writes.push(Write {
                        path: format!("ifaces/{}/addr/set", iface.name),
                        value: format!("{address}\n"),
                    });
                }

                if let Some(gateway) = iface.gateway {
                    writes.push(self.route_write(None, gateway));
                }

                for route in &iface.routes {
                    if let Some(destination) = route.destination {
                        writes.push(self.route_write(Some(destination), route.gateway));
                    }
                }
            }

            if let Some(dns) = iface.dns.first() {
                writes.push(Write {
                    path: "resolv/nameserver".to_owned(),
                    value: format!("{dns}\n"),
                });
            }
        }

        writes
    }

    /// The write that adds a route to the stack.
    ///
    /// The route table of the stack is not per interface, so the route names
    /// its next hop and nothing else.
    fn route_write(&self, destination: Option<Ipv4Cidr>, gateway: Ipv4Addr) -> Write {
        let destination = match destination {
            Some(destination) => destination.to_string(),
            None => "default".to_owned(),
        };

        Write {
            path: "route/add".to_owned(),
            value: format!("{destination} via {gateway}\n"),
        }
    }

    /// What is worth saying about a plan before it is applied.
    ///
    /// The resolver of the network stack is a single address, and a
    /// configuration that names more than one has to hear about it.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();

        for iface in &self.interfaces {
            if iface.dns.len() > 1 {
                notes.push(format!(
                    "{}: the network stack holds one resolver, using {} out of {}",
                    iface.name,
                    iface.dns[0],
                    iface
                        .dns
                        .iter()
                        .map(|dns| dns.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            if iface.name != "eth0" {
                notes.push(format!(
                    "{}: the network stack only configures eth0, the plan for this interface cannot be applied",
                    iface.name
                ));
            }
        }

        notes
    }

    /// The plan as a report, one interface after the other.
    pub fn render(&self, netcfg_root: &str) -> String {
        let mut out = String::new();

        for iface in &self.interfaces {
            if !out.is_empty() {
                out.push('\n');
            }

            let _ = write!(out, "{}: {}", iface.name, iface.mode.as_str());
            if let Some(description) = &iface.description {
                let _ = write!(out, " ({description})");
            }
            if let Some(mac) = iface.mac {
                let _ = write!(out, " [{mac}]");
            }
            match &iface.source {
                Some(source) => {
                    let _ = writeln!(out, ", from {}", source.display());
                }
                None if iface.configured => out.push_str(", from the configuration\n"),
                None => out.push_str(", no configuration\n"),
            }

            if iface.mode == Mode::Static {
                for address in &iface.addresses {
                    let _ = writeln!(out, "    address {address}");
                }
                if let Some(gateway) = iface.gateway {
                    let _ = writeln!(out, "    gateway {gateway}");
                }
                for route in &iface.routes {
                    match route.destination {
                        Some(destination) => {
                            let _ = writeln!(out, "    route {destination} via {}", route.gateway);
                        }
                        None => {
                            let _ = writeln!(out, "    route default via {}", route.gateway);
                        }
                    }
                }
            }

            for dns in &iface.dns {
                let _ = writeln!(out, "    resolver {dns}");
            }
        }

        let writes = self.writes();
        if !writes.is_empty() {
            out.push('\n');
            for write in &writes {
                let _ = writeln!(out, "{}", write.render(netcfg_root));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetworkFile;

    fn parse(text: &str) -> NetworkFile {
        NetworkFile::parse(text, None).unwrap()
    }

    fn eth0() -> Vec<Observed> {
        vec![Observed {
            name: "eth0".to_owned(),
            mac: Some("52:54:00:12:34:56".parse().unwrap()),
        }]
    }

    #[test]
    fn a_static_configuration_becomes_the_writes_the_stack_expects() {
        let config = Config::from_files(vec![parse(
            "[Match]\n\
             Name=eth0\n\
             \n\
             [Network]\n\
             Address=192.168.1.50/24\n\
             Gateway=192.168.1.1\n\
             DNS=9.9.9.9\n\
             \n\
             [Route]\n\
             Destination=10.0.0.0/8\n\
             Gateway=192.168.1.2\n",
        )]);

        let plan = Plan::build(&config, &eth0());
        assert_eq!(plan.interfaces.len(), 1);
        let iface = &plan.interfaces[0];
        assert_eq!(iface.mode, Mode::Static);
        assert!(iface.is_configured());

        assert_eq!(
            plan.writes(),
            vec![
                Write {
                    path: "ifaces/eth0/addr/set".to_owned(),
                    value: "192.168.1.50/24\n".to_owned(),
                },
                Write {
                    path: "route/add".to_owned(),
                    value: "default via 192.168.1.1\n".to_owned(),
                },
                Write {
                    path: "route/add".to_owned(),
                    value: "10.0.0.0/8 via 192.168.1.2\n".to_owned(),
                },
                Write {
                    path: "resolv/nameserver".to_owned(),
                    value: "9.9.9.9\n".to_owned(),
                },
            ]
        );

        // The writes name the control files of the stack, one by one.
        assert_eq!(
            plan.writes()[0].full_path("/scheme/netcfg"),
            "/scheme/netcfg/ifaces/eth0/addr/set"
        );
    }

    #[test]
    fn a_dhcp_interface_is_not_addressed_here() {
        let config = Config::from_files(vec![parse(
            "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\nDNS=9.9.9.9\n",
        )]);

        let plan = Plan::build(&config, &eth0());
        assert_eq!(plan.interfaces[0].mode, Mode::Dhcp);

        // The address, the route and the resolver come from the server; the
        // resolver written down here is the one decision left to apply.
        assert_eq!(
            plan.writes(),
            vec![Write {
                path: "resolv/nameserver".to_owned(),
                value: "9.9.9.9\n".to_owned(),
            }]
        );
    }

    #[test]
    fn an_interface_with_no_file_is_left_to_dhcp() {
        let plan = Plan::build(&Config::new(), &eth0());

        assert_eq!(plan.interfaces[0].mode, Mode::Dhcp);
        assert!(!plan.interfaces[0].is_configured());
        assert!(plan.writes().is_empty());
        assert_eq!(plan.warnings.len(), 1);
        assert!(
            plan.warnings[0].contains("no configuration matches"),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn the_file_is_picked_by_name_and_by_hardware_address() {
        let config = Config::from_files(vec![
            parse("[Match]\nName=wlan*\n\n[Network]\nDHCP=yes\n"),
            parse("[Match]\nMACAddress=52:54:00:12:34:56\n\n[Network]\nAddress=10.0.0.2/8\n"),
        ]);

        let plan = Plan::build(&config, &eth0());
        assert_eq!(plan.interfaces[0].mode, Mode::Static);
        assert_eq!(plan.interfaces[0].addresses[0].to_string(), "10.0.0.2/8");
    }

    #[test]
    fn a_plan_says_what_cannot_be_applied() {
        let config = Config::from_files(vec![parse(
            "[Match]\nName=wlan0\n\n[Network]\nAddress=10.0.0.2/8\nDNS=9.9.9.9 1.1.1.1\n",
        )]);

        let plan = Plan::build(
            &config,
            &[Observed {
                name: "wlan0".to_owned(),
                mac: None,
            }],
        );

        let notes = plan.notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("one resolver"), "{notes:?}");
        assert!(notes[1].contains("only configures eth0"), "{notes:?}");
        assert_eq!(plan.writes().len(), 2, "the first resolver is still set");
    }

    #[test]
    fn a_report_lists_the_interfaces_and_the_writes() {
        let config = Config::from_files(vec![parse(
            "[Match]\nName=eth0\n\n[Network]\nDescription=Ethernet\nAddress=192.168.1.50/24\nGateway=192.168.1.1\n",
        )]);

        let report = Plan::build(&config, &eth0()).render("/scheme/netcfg");
        let expected = [
            "eth0: static (Ethernet) [52:54:00:12:34:56], from the configuration",
            "    address 192.168.1.50/24",
            "    gateway 192.168.1.1",
            "",
            "write /scheme/netcfg/ifaces/eth0/addr/set 192.168.1.50/24",
            "write /scheme/netcfg/route/add default via 192.168.1.1",
            "",
        ]
        .join("\n");

        assert_eq!(report, expected);
    }

    #[test]
    fn a_report_names_the_file_an_interface_came_from() {
        let file = NetworkFile::parse(
            "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
            Some("/etc/networkd/10-eth0.network".into()),
        )
        .unwrap();
        let config = Config::from_files(vec![file]);

        let report = Plan::build(&config, &eth0()).render("/scheme/netcfg");
        assert!(
            report.starts_with(
                "eth0: dhcp [52:54:00:12:34:56], from /etc/networkd/10-eth0.network\n"
            ),
            "{report}"
        );
    }

    #[test]
    fn a_report_says_when_no_file_matched() {
        let report = Plan::build(&Config::new(), &eth0()).render("/scheme/netcfg");
        assert_eq!(report, "eth0: dhcp [52:54:00:12:34:56], no configuration\n");
    }

    #[test]
    fn the_alignment_of_the_commands_does_not_depend_on_the_root() {
        let write = Write {
            path: "route/add".to_owned(),
            value: "default via 10.0.0.1\n".to_owned(),
        };
        assert_eq!(
            write.render("/scheme/netcfg/"),
            "write /scheme/netcfg/route/add default via 10.0.0.1"
        );
        assert_eq!(
            write.render("/netcfg"),
            "write /netcfg/route/add default via 10.0.0.1"
        );
    }
}
