//! End-to-end test of the network pipeline: a configuration directory is read,
//! resolved against the interfaces the network stack serves, turned into the
//! writes the stack understands, and applied by the real `networkctl`.
//!
//! The network stack is a file tree here, with the control files a static
//! address needs: that is exactly what `/scheme/netcfg` looks like to the
//! daemon, so the writes can be checked on the host instead of on the machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use networkd::config::Config;
use networkd::netcfg::Netcfg;
use networkd::plan::{Mode, Observed, Plan};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("networkd-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The control files a stack with `eth0` serves.
fn netcfg_tree(name: &str, mac: &str) -> (PathBuf, Netcfg) {
    let dir = scratch(name);
    for path in [
        "ifaces/eth0/mac",
        "ifaces/eth0/addr/set",
        "route/add",
        "resolv/nameserver",
    ] {
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "").unwrap();
    }
    fs::write(dir.join("ifaces/eth0/mac"), format!("{mac}\n")).unwrap();

    let netcfg = Netcfg::new(dir.to_string_lossy().into_owned());
    (dir, netcfg)
}

fn config_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch(name);
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn networkctl(config: &Path, netcfg: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_networkctl"))
        .env_remove("NETWORKD_CONFIG")
        .env_remove("NETWORKD_NETCFG")
        .arg("--config-dir")
        .arg(config)
        .arg("--netcfg")
        .arg(netcfg)
        .args(args)
        .output()
        .expect("networkctl should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_static_interface_is_configured_and_applied_by_the_real_tool() {
    let config = config_dir(
        "static-config",
        &[(
            "10-eth0.network",
            "[Match]\n\
             Name=eth0\n\
             \n\
             [Network]\n\
             Description=Ethernet\n\
             Address=192.168.1.50/24\n\
             Gateway=192.168.1.1\n\
             DNS=9.9.9.9\n\
             \n\
             [Route]\n\
             Destination=10.0.0.0/8\n\
             Gateway=192.168.1.2\n",
        )],
    );
    let (tree, _) = netcfg_tree("static-stack", "52:54:00:12:34:56");

    // `list` says what the stack serves and what the configuration makes of it.
    let output = networkctl(&config, &tree, &["list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let listing = stdout(&output);
    assert!(listing.starts_with("INTERFACE MODE"), "{listing}");
    assert!(listing.contains("eth0      static"), "{listing}");
    assert!(listing.contains("192.168.1.50/24"), "{listing}");

    // `apply --dry-run` prints the writes and leaves the stack alone.
    let output = networkctl(&config, &tree, &["apply", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let plan = stdout(&output);
    assert!(
        plan.contains("ifaces/eth0/addr/set 192.168.1.50/24"),
        "{plan}"
    );
    assert!(plan.contains("route/add default via 192.168.1.1"), "{plan}");
    assert!(
        plan.contains("route/add 10.0.0.0/8 via 192.168.1.2"),
        "{plan}"
    );
    assert!(plan.contains("resolv/nameserver 9.9.9.9"), "{plan}");
    assert_eq!(
        fs::read_to_string(tree.join("ifaces/eth0/addr/set")).unwrap(),
        "",
        "a dry run does not write"
    );

    // `apply` writes them, one control file after the other.
    let output = networkctl(&config, &tree, &["apply"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read_to_string(tree.join("ifaces/eth0/addr/set")).unwrap(),
        "192.168.1.50/24\n"
    );
    assert_eq!(
        fs::read_to_string(tree.join("resolv/nameserver")).unwrap(),
        "9.9.9.9\n"
    );
    // A control file that takes one value per write shows the last one when it
    // is a plain file: the real `route/add` adds a route on every open, which
    // is why the dry run above is what says both routes are written.
    assert_eq!(
        fs::read_to_string(tree.join("route/add")).unwrap(),
        "10.0.0.0/8 via 192.168.1.2\n"
    );

    // `status` reports the same plan, with the writes at the end.
    let output = networkctl(&config, &tree, &["status", "eth0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let status = stdout(&output);
    assert!(status.starts_with("eth0: static (Ethernet)"), "{status}");
    assert!(status.contains("    address 192.168.1.50/24\n"), "{status}");
    assert!(
        status.contains("    route 10.0.0.0/8 via 192.168.1.2\n"),
        "{status}"
    );

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn an_interface_with_no_file_is_left_to_dhcp() {
    let config = config_dir(
        "dhcp-config",
        &[(
            "20-wlan0.network",
            "[Match]\nName=wlan0\n\n[Network]\nDHCP=yes\n",
        )],
    );
    let (tree, _) = netcfg_tree("dhcp-stack", "52:54:00:12:34:56");

    let output = networkctl(&config, &tree, &["status", "eth0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let status = stdout(&output);
    assert!(status.starts_with("eth0: dhcp"), "{status}");
    assert!(status.contains("no configuration"), "{status}");
    assert!(
        stderr(&output).contains("eth0: no configuration matches, using DHCP"),
        "{}",
        stderr(&output)
    );

    // Nothing is written for an interface that gets everything from a server.
    let output = networkctl(&config, &tree, &["apply"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("nothing to apply"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        fs::read_to_string(tree.join("ifaces/eth0/addr/set")).unwrap(),
        ""
    );

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn the_file_that_matches_first_wins() {
    let config = config_dir(
        "order-config",
        &[
            (
                "10-wildcard.network",
                "[Match]\nName=eth*\n\n[Network]\nDHCP=yes\n",
            ),
            (
                "20-eth0.network",
                "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/24\n",
            ),
        ],
    );
    let (tree, _) = netcfg_tree("order-stack", "52:54:00:12:34:56");

    let output = networkctl(&config, &tree, &["status", "eth0"]);
    let status = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        status.contains("from ") && status.contains("10-wildcard.network"),
        "{status}"
    );

    // The plan of the second file is not applied on top of the first one.
    let output = networkctl(&config, &tree, &["apply", "--dry-run"]);
    assert!(
        stdout(&output).contains("nothing to apply"),
        "{}",
        stdout(&output)
    );

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn an_interface_that_is_not_there_is_an_error() {
    let config = config_dir("missing-config", &[]);
    let (tree, _) = netcfg_tree("missing-stack", "52:54:00:12:34:56");

    let output = networkctl(&config, &tree, &["status", "enp0s3"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no interface named enp0s3"),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("eth0"), "{}", stderr(&output));

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn a_stack_that_is_not_running_is_reported() {
    let config = config_dir("no-stack-config", &[]);
    let missing = scratch("no-stack");

    let output = networkctl(&config, &missing, &["list"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot read the interfaces"),
        "{}",
        stderr(&output)
    );

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&missing);
}

#[test]
fn the_configuration_is_shown_as_it_was_understood() {
    let config = config_dir(
        "show-config",
        &[(
            "10-eth0.network",
            "[Match]\n\
             Name=eth0\n\
             \n\
             [Network]\n\
             Address=10.0.0.2/24\n\
             DNS=9.9.9.9\n\
             \n\
             [Wireless]\n\
             SSID=home\n",
        )],
    );
    let (tree, _) = netcfg_tree("show-stack", "52:54:00:12:34:56");

    let output = networkctl(&config, &tree, &["show"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let shown = stdout(&output);
    assert!(shown.contains("# "), "{shown}");
    assert!(shown.contains("Address=10.0.0.2/24"), "{shown}");
    assert!(
        stderr(&output).contains("unknown section [Wireless]"),
        "{}",
        stderr(&output)
    );

    // `show FILE` reads one file instead of the directory.
    let output = networkctl(
        &config,
        &tree,
        &["show", config.join("10-eth0.network").to_str().unwrap()],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("DNS=9.9.9.9"),
        "{}",
        stdout(&output)
    );

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn a_warning_is_said_once_on_the_error_stream() {
    let config = config_dir(
        "warn-config",
        &[(
            "10-eth0.network",
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/24\n\n[Wireless]\nSSID=home\n",
        )],
    );
    let (tree, _) = netcfg_tree("warn-stack", "52:54:00:12:34:56");

    for args in [
        vec!["list"],
        vec!["status", "eth0"],
        vec!["apply", "--dry-run"],
    ] {
        let output = networkctl(&config, &tree, &args);
        assert!(output.status.success(), "{}", stderr(&output));

        let reported = stderr(&output);
        assert_eq!(
            reported.matches("unknown section [Wireless]").count(),
            1,
            "{args:?}: {reported}"
        );
        assert!(
            !stdout(&output).contains("warning"),
            "{args:?}: the report is not where a warning goes: {}",
            stdout(&output)
        );
    }

    let _ = fs::remove_dir_all(&config);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn the_library_and_the_tool_agree_on_the_same_configuration() {
    let dir = config_dir(
        "agree-config",
        &[(
            "10-eth0.network",
            "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/24\nGateway=10.0.0.1\n",
        )],
    );
    let (tree, netcfg) = netcfg_tree("agree-stack", "52:54:00:12:34:56");

    // The plan, built in process.
    let config = Config::load(&dir).unwrap();
    let observed = netcfg.interfaces().unwrap();
    assert_eq!(
        observed,
        vec![Observed {
            name: "eth0".to_owned(),
            mac: Some("52:54:00:12:34:56".parse().unwrap()),
        }]
    );

    let plan = Plan::build(&config, &observed);
    assert_eq!(plan.interfaces[0].mode, Mode::Static);
    assert_eq!(plan.writes().len(), 2, "an address and a route");

    // The tool prints exactly the writes the plan holds, in the same order.
    let output = networkctl(&dir, &tree, &["apply", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let expected: String = plan
        .writes()
        .iter()
        .map(|write| format!("{}\n", write.render(netcfg.root())))
        .collect();
    assert_eq!(stdout(&output), expected);

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&tree);
}
