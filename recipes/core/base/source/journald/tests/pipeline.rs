//! End-to-end test of the journal pipeline: raw log lines are parsed into
//! entries, persisted to a file, read back, filtered through the `journalctl`
//! argument parser and rendered.

use std::fs;
use std::path::PathBuf;

use journald::cli::{parse_args, OutputMode};
use journald::follow::Follower;
use journald::priority::Priority;
use journald::query::Query;
use journald::render::{render, render_boot_list};
use journald::store::JournalStore;
use journald::syslog::Source;
use journald::writer::{JournalConfig, JournalWriter};

const BOOT_ID: &str = "00000000deadbeef123456789abcdef0";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("journald-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_sample_journal(path: &std::path::Path) {
    let source = Source {
        boot_id: Some(BOOT_ID.to_owned()),
        hostname: Some("aios".to_owned()),
        now: 1_792_022_400,
        ..Source::default()
    };

    let lines = [
        "<6>init[1]: Starting 10_smolnetd.service",
        "smolnetd: interface eth0 up",
        "<4>smolnetd: link down on eth0",
        "<3>dhcpd: DHCP offer timed out",
        "<6>getty[42]: welcome",
    ];

    for line in lines {
        let entry = journald::syslog::parse_line(line, &source);
        JournalStore::append_to_path(path, &entry).unwrap();
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn pipeline_from_lines_to_rendered_output() {
    let dir = scratch("pipeline");
    let path = dir.join("journal");
    write_sample_journal(&path);

    let store = JournalStore::read_from_path(&path).unwrap();
    assert_eq!(store.len(), 5);
    assert_eq!(store.boot_ids(), vec![BOOT_ID]);

    // `journalctl -u smolnetd`
    let cli = parse_args(args(&["-u", "smolnetd"]), 1_792_022_400).unwrap();
    let matched = store.query(&cli.query);
    let rendered = render(&matched, OutputMode::Short, "aios");
    assert_eq!(matched.len(), 2);
    assert!(rendered.contains("smolnetd: interface eth0 up"));
    assert!(rendered.contains("Oct 15 00:00:00 aios smolnetd: link down on eth0"));
    assert!(!rendered.contains("welcome"));

    // `journalctl -p err` keeps only critical and worse.
    let cli = parse_args(args(&["-p", "err"]), 1_792_022_400).unwrap();
    let errors = store.query(&cli.query);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message(), Some("DHCP offer timed out"));

    // `journalctl -n 2` shows the newest two entries.
    let cli = parse_args(args(&["-n", "2"]), 1_792_022_400).unwrap();
    let tail = store.query_tail(&cli.query, cli.lines.unwrap());
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].message(), Some("DHCP offer timed out"));
    assert_eq!(tail[1].message(), Some("welcome"));

    // Time windows and free-text search combine.
    let cli = parse_args(args(&["--since", "-1s", "-g", "DOWN"]), 1_792_022_400).unwrap();
    assert_eq!(store.query(&cli.query).len(), 1);

    // `journalctl -b` selects the current boot.
    let cli = parse_args(args(&["-b"]), 1_792_022_400).unwrap();
    assert_eq!(store.query(&cli.query).len(), 5);

    // Export output parses back into an identical journal.
    let cli = parse_args(args(&["-o", "export"]), 1_792_022_400).unwrap();
    let exported = render(&store.query(&cli.query), cli.output, "aios");
    let reparsed = JournalStore::decode(&exported).unwrap();
    assert_eq!(reparsed.len(), store.len());
    assert_eq!(reparsed.entries(), store.entries());

    // JSON output is machine readable.
    let cli = parse_args(args(&["-o", "json", "-u", "getty"]), 1_792_022_400).unwrap();
    let json = render(&store.query(&cli.query), cli.output, "aios");
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(value["MESSAGE"], "welcome");
    assert_eq!(value["_PID"], "42");
    assert_eq!(value["_BOOT_ID"], BOOT_ID);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn boot_listing_from_a_persisted_journal() {
    let dir = scratch("boots");
    let path = dir.join("journal");

    for (boot, message) in [
        ("00000000deadbeef123456789abcde01", "first boot line"),
        ("00000000deadbeef123456789abcde02", "second boot line"),
        ("00000000deadbeef123456789abcde02", "second boot again"),
    ] {
        let source = Source {
            boot_id: Some(boot.to_owned()),
            hostname: Some("aios".to_owned()),
            now: 1_792_022_400,
            ..Source::default()
        };
        let entry = journald::syslog::parse_line(message, &source);
        JournalStore::append_to_path(&path, &entry).unwrap();
    }

    let store = JournalStore::read_from_path(&path).unwrap();
    let listing = render_boot_list(&store.boot_summary());
    assert!(listing.contains("00000000deadbeef123456789abcde01"));
    assert!(listing.contains("00000000deadbeef123456789abcde02"));
    // The two boots have one and two entries respectively.
    assert_eq!(listing.lines().count(), 3);
    assert!(listing.contains("  1\n"));
    assert!(listing.contains("  2\n"));

    // Selecting the second boot by identifier returns only its entries.
    let cli = parse_args(
        args(&["--boot=00000000deadbeef123456789abcde02"]),
        1_792_022_400,
    )
    .unwrap();
    assert_eq!(store.query(&cli.query).len(), 2);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn vacuuming_keeps_the_journal_bounded_on_disk() {
    let dir = scratch("vacuum");
    let path = dir.join("journal");

    let mut store = JournalStore::new();
    for index in 0..10 {
        let source = Source {
            boot_id: Some(BOOT_ID.to_owned()),
            hostname: Some("aios".to_owned()),
            now: 1_792_022_400 + index,
            ..Source::default()
        };
        store.append_line(&format!("logd[1]: entry {index}"), &source);
    }

    store.vacuum_before(1_792_022_405);
    assert_eq!(store.len(), 5);
    store.write_to_path(&path).unwrap();

    let reloaded = JournalStore::read_from_path(&path).unwrap();
    assert_eq!(reloaded.len(), 5);
    assert_eq!(reloaded.entries()[0].message(), Some("entry 5"));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_follower_sees_exactly_what_the_writer_appends() {
    let dir = scratch("follow-e2e");
    let path = dir.join("journal");

    let mut writer = JournalWriter::new(JournalConfig {
        path: path.clone(),
        max_bytes: 4096,
        boot_id: None,
        hostname: Some("aios".to_owned()),
    });

    // The daemon writes the boot line before anyone follows.
    assert!(writer.write_line("<6>init[1]: booting"));

    let mut follower = Follower::new(&path, Query::new(), OutputMode::Short, "aios").unwrap();
    let mut out = Vec::new();
    assert_eq!(
        follower.render(&mut out).unwrap(),
        0,
        "the backlog must not be replayed by --follow"
    );

    // New lines reach the follower, without their CRLF ending.
    assert!(writer.write_line("<4>smolnetd: link down on eth0\r\n"));
    assert_eq!(follower.render(&mut out).unwrap(), 1);
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("link down on eth0"));
    assert!(!rendered.contains("booting"));

    // Queries apply while following too.
    let mut only_errors = Follower::new(
        &path,
        Query {
            priority: Some(Priority::Err),
            ..Query::new()
        },
        OutputMode::Cat,
        "aios",
    )
    .unwrap();
    writer.write_line("<6>dhcpd: offer sent");
    writer.write_line("<3>dhcpd: offer timed out");

    let mut errors = Vec::new();
    assert_eq!(only_errors.render(&mut errors).unwrap(), 1);
    assert_eq!(String::from_utf8(errors).unwrap(), "offer timed out\n");

    // What the daemon wrote is still an ordinary journal: one boot, four lines.
    let store = JournalStore::read_from_path(&path).unwrap();
    assert_eq!(store.len(), 4);
    assert_eq!(store.boot_ids().len(), 1);
    assert_eq!(store.current_boot(), Some(writer.boot_id()));

    // `journalctl -b` on the file selects that boot.
    let cli = parse_args(args(&["-b"]), 1_792_022_400).unwrap();
    assert_eq!(store.query(&cli.query).len(), 4);

    fs::remove_dir_all(&dir).unwrap();
}
