//! Output for `networkctl`.
//!
//! Listings are aligned tables, and the detail of a plan is written the way the
//! control files are: one line per write, spelled out, so what is about to be
//! applied can be read and checked before it happens.

use crate::config::Config;
use crate::plan::{InterfacePlan, Plan};
use crate::protocol::Outcome;

const INTERFACE_COLUMNS: [&str; 5] = ["INTERFACE", "MODE", "ADDRESS", "GATEWAY", "DNS"];

/// Renders a row of aligned columns.
fn row(values: &[String], widths: &[usize]) -> String {
    let mut line = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            line.push(' ');
        }
        line.push_str(&format!("{value:<width$}", width = widths[index]));
    }
    line.trim_end().to_owned()
}

fn widths(columns: &[&str], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = columns.iter().map(|column| column.len()).collect();
    for values in rows {
        for (index, value) in values.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(value.chars().count());
            }
        }
    }
    widths
}

fn list(values: &[impl std::fmt::Display], empty: &str) -> String {
    if values.is_empty() {
        empty.to_owned()
    } else {
        values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `networkctl list`: one line per interface the stack serves.
pub fn render_interfaces(plan: &Plan, no_legend: bool) -> String {
    let rows: Vec<Vec<String>> = plan
        .interfaces
        .iter()
        .map(|iface| {
            vec![
                iface.name.clone(),
                iface.mode.as_str().to_owned(),
                list(&iface.addresses, "-"),
                iface
                    .gateway
                    .map(|gateway| gateway.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                list(&iface.dns, "-"),
            ]
        })
        .collect();

    let widths = widths(&INTERFACE_COLUMNS, &rows);
    let mut out = String::new();

    if !no_legend {
        let header: Vec<String> = INTERFACE_COLUMNS
            .iter()
            .map(|column| (*column).to_owned())
            .collect();
        out.push_str(&row(&header, &widths));
        out.push('\n');
    }

    for values in rows {
        out.push_str(&row(&values, &widths));
        out.push('\n');
    }

    out
}

/// `networkctl status`: the plan, and everything worth saying about it.
///
/// A single interface can be asked about: the report of the others is left
/// out, the warnings that come from the configuration are not, because they
/// are about the files and not about one interface.
pub fn render_status(plan: &Plan, interface: Option<&str>, netcfg_root: &str) -> String {
    let selected = match interface {
        None => plan.clone(),
        Some(name) => match plan.interfaces.iter().find(|iface| iface.name == name) {
            Some(iface) => Plan {
                interfaces: vec![iface.clone()],
                warnings: Vec::new(),
            },
            None => return format!("networkctl: {}", unknown_interface(name, plan)),
        },
    };

    let mut out = selected.render(netcfg_root);

    let notes = selected.notes();
    if !notes.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }

        for note in &notes {
            out.push_str(&format!("note: {note}\n"));
        }
    }

    out
}

/// What `networkctl status <name>` says about an interface that is not there.
pub fn unknown_interface(name: &str, plan: &Plan) -> String {
    let known: Vec<&str> = plan
        .interfaces
        .iter()
        .map(|iface| iface.name.as_str())
        .collect();

    if known.is_empty() {
        "no interface is served by the network stack\n".to_owned()
    } else {
        format!(
            "no interface named {name}; the stack serves {}\n",
            known.join(", ")
        )
    }
}

/// `networkctl show`: the configuration as it was understood.
pub fn render_config(config: &Config) -> String {
    let mut out = String::new();

    for file in config.files() {
        let name = file
            .source
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<configuration>".to_owned());
        out.push_str(&format!("# {name}\n"));
        out.push_str(&Config::from_files(vec![file.clone()]).render());
    }

    if out.is_empty() {
        out.push_str("networkctl: no configuration file was found\n");
    }

    out
}

/// What was applied, one write per line.
pub fn render_applied(applied: &[String]) -> String {
    let mut out = String::new();
    for write in applied {
        out.push_str(write);
        out.push('\n');
    }
    out
}

/// What an application the daemon answered looks like.
///
/// The daemon is the one that wrote, and the stack it wrote is in the answer:
/// saying which stack it was is what lets a reader notice that the daemon is not
/// managing the stack they asked about.
pub fn render_delegated(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Applied { writes, netcfg, .. } => format!(
            "applied {writes} {} to {} (the network daemon)\n",
            if *writes == 1 { "write" } else { "writes" },
            netcfg.display()
        ),
        Outcome::Unchanged { netcfg, .. } => format!(
            "{}: nothing to write (the network daemon)\n",
            netcfg.display()
        ),
    }
}

/// The live report, the way the daemon serves it as the file `status`.
///
/// What is wrong with the configuration is part of the report here, because a
/// reader of the file has no other place to see it: a warning would otherwise be
/// lost between the daemon and whoever looks at it. Whether the configuration
/// has already been written to the stack is part of it for the same reason.
pub fn render_status_document(plan: &Plan, netcfg_root: &str, applied: bool) -> String {
    let mut out = render_status(plan, None, netcfg_root);

    for warning in &plan.warnings {
        out.push_str(&format!("warning: {warning}\n"));
    }

    out.push_str(if applied {
        "applied: yes\n"
    } else {
        "applied: no\n"
    });

    out
}

/// A one line summary of an interface, for the last line of a status.
pub fn summarize(iface: &InterfacePlan) -> String {
    format!(
        "{}: {} ({} addresses, {} routes, {} resolvers)",
        iface.name,
        iface.mode.as_str(),
        iface.addresses.len(),
        iface.routes.len() + usize::from(iface.gateway.is_some()),
        iface.dns.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetworkFile;
    use crate::plan::Observed;

    fn plan(text: &str) -> Plan {
        let config = Config::from_files(vec![NetworkFile::parse(text, None).unwrap()]);
        Plan::build(
            &config,
            &[
                Observed {
                    name: "eth0".to_owned(),
                    mac: Some("52:54:00:12:34:56".parse().unwrap()),
                },
                Observed {
                    name: "wlan0".to_owned(),
                    mac: None,
                },
            ],
        )
    }

    #[test]
    fn a_listing_aligns_the_interfaces() {
        let plan = plan(
            "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nGateway=192.168.1.1\nDNS=9.9.9.9\n",
        );

        let listing = render_interfaces(&plan, false);
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 3, "{listing}");

        // Every column starts at the same place, whatever the row.
        let header = lines[0];
        assert!(header.starts_with("INTERFACE MODE"), "{header}");
        assert_eq!(
            header.find("GATEWAY"),
            lines[1].find("192.168.1.1"),
            "{listing}"
        );

        assert!(lines[1].starts_with("eth0      static"), "{listing}");
        assert!(lines[1].contains("192.168.1.50/24"), "{listing}");
        assert!(lines[1].contains("9.9.9.9"), "{listing}");
        assert!(lines[2].starts_with("wlan0     dhcp"), "{listing}");
        assert!(lines[2].ends_with("-"), "{listing}");
    }

    #[test]
    fn a_listing_can_leave_the_header_out() {
        let plan = plan("[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n");
        let listing = render_interfaces(&plan, true);
        assert!(!listing.contains("INTERFACE"), "{listing}");
        assert!(listing.starts_with("eth0"), "{listing}");
    }

    #[test]
    fn a_status_can_be_asked_for_one_interface() {
        let plan = plan("[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n");
        let status = render_status(&plan, Some("wlan0"), "/scheme/netcfg");

        assert!(status.starts_with("wlan0: dhcp"), "{status}");
        assert!(!status.contains("eth0:"), "{status}");
    }

    #[test]
    fn an_interface_that_is_not_there_is_named() {
        let plan = plan("[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n");
        let message = render_status(&plan, Some("enp0s3"), "/scheme/netcfg");

        assert!(
            message.starts_with("networkctl: no interface named enp0s3"),
            "{message}"
        );
        assert!(message.contains("eth0, wlan0"), "{message}");
    }

    #[test]
    fn a_status_carries_the_notes_of_the_plan() {
        let plan = plan("[Match]\nName=eth0\n\n[Network]\nDHCP=yes\nDNS=9.9.9.9 1.1.1.1\n");
        let status = render_status(&plan, None, "/scheme/netcfg");

        assert!(
            status.contains("note: eth0: the network stack holds one resolver"),
            "{status}"
        );
        assert!(
            status.contains("note: wlan0: the network stack only configures eth0"),
            "{status}"
        );
    }

    #[test]
    fn a_configuration_is_shown_with_the_file_it_came_from() {
        let config = Config::from_files(vec![NetworkFile::parse(
            "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
            Some("/etc/networkd/10-eth0.network".into()),
        )
        .unwrap()]);

        let shown = render_config(&config);
        assert!(
            shown.starts_with("# /etc/networkd/10-eth0.network\n"),
            "{shown}"
        );
        assert!(shown.contains("[Match]\nName=eth0\n"), "{shown}");
    }

    #[test]
    fn a_system_with_no_configuration_says_so() {
        let shown = render_config(&Config::new());
        assert_eq!(shown, "networkctl: no configuration file was found\n");
    }

    #[test]
    fn an_interface_is_summarized_in_one_line() {
        let plan = plan(
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\nGateway=10.0.0.1\nDNS=9.9.9.9\n",
        );
        assert_eq!(
            summarize(&plan.interfaces[0]),
            "eth0: static (1 addresses, 1 routes, 1 resolvers)"
        );
    }

    #[test]
    fn a_write_the_daemon_answered_says_which_stack_it_wrote() {
        let applied = |writes| Outcome::Applied {
            writes,
            netcfg: "/scheme/netcfg".into(),
            config: "/etc/networkd".into(),
        };

        assert_eq!(
            render_delegated(&applied(1)),
            "applied 1 write to /scheme/netcfg (the network daemon)\n"
        );
        assert_eq!(
            render_delegated(&applied(3)),
            "applied 3 writes to /scheme/netcfg (the network daemon)\n"
        );

        let unchanged = Outcome::Unchanged {
            netcfg: "/scheme/netcfg".into(),
            config: "/etc/networkd".into(),
        };
        assert_eq!(
            render_delegated(&unchanged),
            "/scheme/netcfg: nothing to write (the network daemon)\n"
        );
    }

    #[test]
    fn the_report_the_daemon_serves_carries_what_is_wrong_with_it() {
        // Only `wlan0` is configured, so `eth0` is worth a warning: a reader of
        // the report has no other place to see it.
        let plan = plan("[Match]\nName=wlan0\n\n[Network]\nDHCP=yes\n");

        let report = render_status_document(&plan, "/scheme/netcfg", false);
        assert!(report.contains("wlan0: dhcp, from"), "{report}");
        assert!(
            report.contains("warning: eth0: no configuration matches, using DHCP\n"),
            "{report}"
        );
        assert!(report.ends_with("applied: no\n"), "{report}");

        let report = render_status_document(&plan, "/scheme/netcfg", true);
        assert!(report.ends_with("applied: yes\n"), "{report}");
    }
}
