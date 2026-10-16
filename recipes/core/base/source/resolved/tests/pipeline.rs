//! End-to-end test of the resolver pipeline: the resolver the network stack
//! holds is read, written to the file a program reads, and the file is the one
//! the C library would take its `nameserver` from.
//!
//! The network stack is a file tree here, with the one control file the
//! resolver lives in, and the file is a file in a temporary directory: that is
//! exactly what `/scheme/netcfg` and `/etc/resolv.conf` look like to the
//! command, so the whole path can be checked on the host instead of on the
//! machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resolved-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A network stack serving the control file the resolver is written to.
fn netcfg_tree(name: &str, resolver: Option<&str>) -> PathBuf {
    let dir = scratch(&format!("{name}-netcfg")).join("netcfg");
    let control = dir.join("resolv/nameserver");
    fs::create_dir_all(control.parent().unwrap()).unwrap();
    fs::write(&control, resolver.unwrap_or("")).unwrap();
    dir
}

/// A file to manage, written before the command runs.
fn resolv_file(name: &str, text: Option<&str>) -> PathBuf {
    let file = scratch(&format!("{name}-file")).join("resolv.conf");
    if let Some(text) = text {
        fs::write(&file, text).unwrap();
    }
    file
}

fn resolvectl(file: &Path, netcfg: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_resolvectl"))
        .env_remove("RESOLVED_FILE")
        .env_remove("NETWORKD_NETCFG")
        .arg("--file")
        .arg(file)
        .arg("--netcfg")
        .arg(netcfg)
        .args(args)
        .output()
        .expect("resolvectl should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_resolver_of_the_stack_ends_up_in_the_file_a_program_reads() {
    let netcfg = netcfg_tree("learned", Some("1.1.1.1\n"));
    let file = resolv_file("learned", None);

    // Nothing wrote the file: that is the state this crate exists for.
    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(status.status.success(), "{}", stderr(&status));
    let text = stdout(&status);
    assert!(text.contains("(not written yet)"), "{text}");
    assert!(text.contains(" Resolvers: none\n"), "{text}");
    assert!(text.contains("     Stack: 1.1.1.1\n"), "{text}");
    assert!(text.contains("   In sync: no\n"), "{text}");
    assert!(!file.exists());
    assert!(!stderr(&status).contains("warning:"), "{}", stderr(&status));

    // A dry run says what would be written, and writes nothing.
    let dry = resolvectl(&file, &netcfg, &["update", "--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(stdout(&dry).starts_with("would write "), "{}", stdout(&dry));
    assert!(
        stdout(&dry).contains("nameserver 1.1.1.1\n"),
        "{}",
        stdout(&dry)
    );
    assert!(!file.exists(), "a dry run writes nothing");

    // And then the file is there, with the line the library reads first.
    let update = resolvectl(&file, &netcfg, &["update"]);
    assert!(update.status.success(), "{}", stderr(&update));
    assert!(stdout(&update).starts_with("wrote "), "{}", stdout(&update));

    let written = fs::read_to_string(&file).unwrap();
    assert_eq!(
        written.lines().find(|line| line.starts_with("nameserver ")),
        Some("nameserver 1.1.1.1"),
        "{written}"
    );

    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(
        stdout(&status).contains("   In sync: yes\n"),
        "{}",
        stdout(&status)
    );

    // A second update has nothing to do.
    let again = resolvectl(&file, &netcfg, &["update"]);
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(
        stdout(&again).contains("already in sync"),
        "{}",
        stdout(&again)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), written);

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}

#[test]
fn what_the_file_says_beyond_the_resolver_survives_the_update() {
    let netcfg = netcfg_tree("keep", Some("9.9.9.9\n"));
    let file = resolv_file(
        "keep",
        Some("# written by hand\nnameserver 8.8.8.8\nsearch example.org local\noptions ndots:1\nsortlist 10.0.0.0/255.0.0.0\n"),
    );

    let update = resolvectl(&file, &netcfg, &["update"]);
    assert!(update.status.success(), "{}", stderr(&update));

    let written = fs::read_to_string(&file).unwrap();
    assert!(written.contains("nameserver 9.9.9.9\n"), "{written}");
    assert!(!written.contains("8.8.8.8"), "{written}");
    assert!(written.contains("search example.org local\n"), "{written}");
    assert!(written.contains("options ndots:1\n"), "{written}");
    assert!(
        written.contains("sortlist 10.0.0.0/255.0.0.0\n"),
        "a line that is not understood is not a line to drop: {written}"
    );

    // What is not understood is reported, and the rest is still right.
    let stderr = stderr(&update);
    assert!(stderr.contains("line 5: unknown directive"), "{stderr}");
    assert!(stderr.contains("sortlist"), "{stderr}");

    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(
        stdout(&status).contains("    Search: example.org local\n"),
        "{}",
        stdout(&status)
    );
    assert!(
        stdout(&status).contains("   Options: ndots:1\n"),
        "{}",
        stdout(&status)
    );

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}

#[test]
fn the_resolver_can_be_set_in_the_stack_and_in_the_file() {
    let netcfg = netcfg_tree("set", Some("8.8.8.8\n"));
    let file = resolv_file("set", Some("# my file\nnameserver 8.8.8.8\n"));

    let dry = resolvectl(&file, &netcfg, &["set-dns", "1.1.1.1", "--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(stdout(&dry).contains("would write "), "{}", stdout(&dry));
    assert!(
        stdout(&dry).contains("nameserver 1.1.1.1\n"),
        "{}",
        stdout(&dry)
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "# my file\nnameserver 8.8.8.8\n"
    );
    assert_eq!(
        fs::read_to_string(netcfg.join("resolv/nameserver")).unwrap(),
        "8.8.8.8\n"
    );

    let set = resolvectl(&file, &netcfg, &["set-dns", "1.1.1.1"]);
    assert!(set.status.success(), "{}", stderr(&set));
    let text = stdout(&set);
    assert!(text.contains("resolv/nameserver 1.1.1.1\n"), "{text}");
    assert!(text.contains("wrote "), "{text}");

    // The stack holds it, and the file a program reads says it.
    assert_eq!(
        fs::read_to_string(netcfg.join("resolv/nameserver")).unwrap(),
        "1.1.1.1\n"
    );
    let written = fs::read_to_string(&file).unwrap();
    assert!(written.contains("nameserver 1.1.1.1\n"), "{written}");

    // A comment is not something the file says: the file is regenerated, and
    // the line that says where it comes from is written in its place.
    assert!(
        written.contains("# Generated by resolved"),
        "the file says where it comes from: {written}"
    );

    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(
        stdout(&status).contains("     Stack: 1.1.1.1\n"),
        "{}",
        stdout(&status)
    );
    assert!(
        stdout(&status).contains("   In sync: yes\n"),
        "{}",
        stdout(&status)
    );

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}

#[test]
fn a_resolver_a_query_could_not_be_sent_to_is_refused() {
    let netcfg = netcfg_tree("refused", Some("8.8.8.8\n"));
    let file = resolv_file("refused", None);

    for addr in ["0.0.0.0", "255.255.255.255", "224.0.0.1"] {
        let output = resolvectl(&file, &netcfg, &["set-dns", addr]);
        assert!(!output.status.success(), "{addr} was accepted");
        assert!(
            stderr(&output).contains("not an address a query can be sent to"),
            "{addr}: {}",
            stderr(&output)
        );
    }

    // Nothing moved: neither the stack nor the file.
    assert_eq!(
        fs::read_to_string(netcfg.join("resolv/nameserver")).unwrap(),
        "8.8.8.8\n"
    );
    assert!(!file.exists());

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}

#[test]
fn a_stack_that_is_not_running_is_said_out_loud() {
    let netcfg = scratch("down").join("netcfg"); // never created
    let file = resolv_file("down", Some("nameserver 8.8.8.8\n"));

    // `status` still reads the file, and says the stack is not reachable.
    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(status.status.success(), "{}", stderr(&status));
    let text = stdout(&status);
    assert!(text.contains(" Resolvers: 8.8.8.8\n"), "{text}");
    assert!(text.contains("     Stack: not reachable: "), "{text}");
    assert!(text.contains("   In sync: no\n"), "{text}");

    // An empty stack is not an absent stack: one cannot be updated, the other
    // has to be given a resolver first.
    let output = resolvectl(&file, &netcfg, &["update"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot be read"),
        "{}",
        stderr(&output)
    );

    let empty = netcfg_tree("empty", None);
    let output = resolvectl(&file, &empty, &["update"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("holds no resolver"),
        "{}",
        stderr(&output)
    );

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(empty.parent().unwrap());
}

#[test]
fn dns_lists_what_a_lookup_would_use_and_where_it_is_from() {
    let netcfg = netcfg_tree("dns", Some("9.9.9.9\n"));
    let file = resolv_file("dns", Some("nameserver 9.9.9.9\n"));

    let output = resolvectl(&file, &netcfg, &["dns"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "RESOLVER ORIGIN\n9.9.9.9  file\n");

    let output = resolvectl(&file, &netcfg, &["dns", "--no-legend"]);
    assert_eq!(stdout(&output), "9.9.9.9 file\n");

    // The stack says something else: the listing names it, which is what makes
    // "in sync: no" readable.
    let other = netcfg_tree("dns-other", Some("1.1.1.1\n"));
    let output = resolvectl(&file, &other, &["dns"]);
    let text = stdout(&output);
    assert!(text.contains("9.9.9.9  file\n"), "{text}");
    assert!(text.contains("1.1.1.1  stack\n"), "{text}");

    // No file and no resolver: there is nothing to list.
    let missing = resolv_file("dns-missing", None);
    let empty = netcfg_tree("dns-empty", None);
    let output = resolvectl(&missing, &empty, &["dns"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("no resolver is configured"),
        "{}",
        stdout(&output)
    );

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
    let _ = fs::remove_dir_all(other.parent().unwrap());
    let _ = fs::remove_dir_all(missing.parent().unwrap());
    let _ = fs::remove_dir_all(empty.parent().unwrap());
}

#[test]
fn a_file_that_is_not_understood_is_reported_and_still_read() {
    let netcfg = netcfg_tree("bad", Some("9.9.9.9\n"));
    let file = resolv_file(
        "bad",
        Some("nameserver banana\nnameserver 224.0.0.1\nnameserver 9.9.9.9 1.1.1.1\n"),
    );

    let output = resolvectl(&file, &netcfg, &["status"]);
    assert!(output.status.success(), "{}", stderr(&output));

    let warnings = stderr(&output);
    assert!(
        warnings.contains("line 1: nameserver: banana"),
        "{warnings}"
    );
    assert!(
        warnings.contains("line 2: nameserver 224.0.0.1"),
        "{warnings}"
    );
    assert!(
        warnings.contains("line 3: nameserver takes one value"),
        "{warnings}"
    );

    // The one line that is a resolver is the one used, and the rest of the
    // lines are still in the file.
    let text = stdout(&output);
    assert!(text.contains(" Resolvers: 9.9.9.9\n"), "{text}");

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}

#[test]
fn what_cannot_be_understood_fails_before_anything_is_touched() {
    let netcfg = netcfg_tree("usage", Some("9.9.9.9\n"));
    let file = resolv_file("usage", Some("nameserver 9.9.9.9\n"));

    let help = Command::new(env!("CARGO_BIN_EXE_resolvectl"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(stdout(&help).contains("resolvectl — control the resolver of the system"));
    assert!(stdout(&help).contains("set-dns ADDRESS"));

    let version = Command::new(env!("CARGO_BIN_EXE_resolvectl"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(
        stdout(&version).starts_with("resolvectl "),
        "{}",
        stdout(&version)
    );

    for (args, message) in [
        (vec!["flush-caches"], "unknown command"),
        (vec!["set-dns"], "set-dns needs an address"),
        (vec!["set-dns", "banana"], "not an IPv4 address"),
        (
            vec!["--dry-run", "status"],
            "means something for `update` and `set-dns` only",
        ),
    ] {
        let output = resolvectl(&file, &netcfg, &args);
        assert!(!output.status.success(), "{args:?} was accepted");
        assert!(
            stderr(&output).contains(message),
            "{args:?}: {}",
            stderr(&output)
        );
    }

    // Nothing was written by any of them.
    assert_eq!(fs::read_to_string(&file).unwrap(), "nameserver 9.9.9.9\n");
    assert_eq!(
        fs::read_to_string(netcfg.join("resolv/nameserver")).unwrap(),
        "9.9.9.9\n"
    );

    let _ = fs::remove_dir_all(file.parent().unwrap());
    let _ = fs::remove_dir_all(netcfg.parent().unwrap());
}
