//! Output for `resolvectl`.
//!
//! Every command answers the same question in a different size: `dns` is the
//! one line a lookup uses, `status` is that line and where it comes from, and
//! `update` is the file it becomes.

use crate::state::{Status, Sync};

const DNS_COLUMNS: [&str; 2] = ["RESOLVER", "ORIGIN"];

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
            let width = value.chars().count();
            if index >= widths.len() {
                widths.push(width);
            } else {
                widths[index] = widths[index].max(width);
            }
        }
    }
    widths
}

/// `resolvectl status`: the resolver, and where it comes from.
pub fn render_status(status: &Status) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "      File: {}{}\n",
        status.file.display(),
        if status.exists() {
            ""
        } else {
            " (not written yet)"
        }
    ));

    let resolvers: Vec<String> = status
        .conf
        .nameservers
        .iter()
        .map(|addr| addr.to_string())
        .collect();
    out.push_str(&format!(
        " Resolvers: {}\n",
        if resolvers.is_empty() {
            "none".to_owned()
        } else {
            resolvers.join(" ")
        }
    ));
    out.push_str(&format!("     Stack: {}\n", status.stack));
    out.push_str(&format!(
        "   In sync: {}\n",
        if status.in_sync() { "yes" } else { "no" }
    ));

    if status.conf.has_search() {
        out.push_str(&format!("    Search: {}\n", status.conf.search.join(" ")));
    }
    if !status.conf.options.is_empty() {
        out.push_str(&format!("   Options: {}\n", status.conf.options.join(" ")));
    }

    out
}

/// `resolvectl dns`: the resolvers a lookup would use, and where each is from.
///
/// The resolver of the stack is listed even when the file does not have it yet:
/// that difference is exactly what `status` calls "in sync: no", and the line
/// that names it is what makes the difference readable.
pub fn render_dns(status: &Status, no_legend: bool) -> String {
    let mut rows: Vec<Vec<String>> = status
        .conf
        .nameservers
        .iter()
        .map(|addr| vec![addr.to_string(), "file".to_owned()])
        .collect();

    if let Some(addr) = status.stack.resolver() {
        // The resolver of the stack is the one a lookup uses first; a file that
        // does not say the same thing gets a line saying so.
        if !status.in_sync() {
            rows.push(vec![addr.to_string(), "stack".to_owned()]);
        }
    }

    let widths = if no_legend {
        // The header is not printed, so it does not decide the layout either.
        widths(&[], &rows)
    } else {
        widths(&DNS_COLUMNS, &rows)
    };
    let mut out = String::new();

    if !no_legend {
        let header: Vec<String> = DNS_COLUMNS
            .iter()
            .map(|column| (*column).to_owned())
            .collect();
        out.push_str(&row(&header, &widths));
        out.push('\n');
    }

    if rows.is_empty() {
        out.push_str("no resolver is configured\n");
        return out;
    }

    for values in rows {
        out.push_str(&row(&values, &widths));
        out.push('\n');
    }

    out
}

/// `resolvectl update` and `resolvectl set-dns`: the file that was written.
pub fn render_sync(sync: &Sync, dry_run: bool) -> String {
    if sync.is_noop() {
        return format!("{}: already in sync\n", sync.path.display());
    }

    format!(
        "{} {}:\n{}",
        if dry_run { "would write" } else { "wrote" },
        sync.path.display(),
        sync.rendered
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::resolv::parse;
    use crate::state::Stack;

    fn sample(text: Option<&str>, stack: Stack) -> Status {
        let (conf, warnings) = match text {
            Some(text) => crate::resolv::parse_with_diagnostics(text),
            None => (crate::resolv::ResolvConf::new(), Vec::new()),
        };

        Status {
            file: PathBuf::from("/etc/resolv.conf"),
            text: text.map(str::to_owned),
            conf,
            warnings,
            stack,
        }
    }

    #[test]
    fn a_status_names_the_file_the_resolver_and_the_stack() {
        let status = sample(
            Some("nameserver 9.9.9.9\n"),
            Stack::Resolver("9.9.9.9".parse().unwrap()),
        );

        let expected = [
            "      File: /etc/resolv.conf",
            " Resolvers: 9.9.9.9",
            "     Stack: 9.9.9.9",
            "   In sync: yes",
            "",
        ]
        .join("\n");

        assert_eq!(render_status(&status), expected);
    }

    #[test]
    fn a_status_says_when_the_file_is_not_there_and_when_it_disagrees() {
        let status = sample(None, Stack::Resolver("1.1.1.1".parse().unwrap()));
        let rendered = render_status(&status);
        assert!(
            rendered.contains("/etc/resolv.conf (not written yet)"),
            "{rendered}"
        );
        assert!(rendered.contains(" Resolvers: none\n"), "{rendered}");
        assert!(rendered.contains("   In sync: no\n"), "{rendered}");

        // A stack that is not running says so instead of saying "none".
        let status = sample(
            Some("nameserver 9.9.9.9\n"),
            Stack::Unavailable("cannot read /scheme/netcfg/resolv/nameserver".to_owned()),
        );
        let rendered = render_status(&status);
        assert!(
            rendered.contains("     Stack: not reachable: "),
            "{rendered}"
        );
    }

    #[test]
    fn a_status_shows_the_search_list_and_the_options() {
        let status = sample(
            Some("nameserver 9.9.9.9\nsearch example.org local\noptions ndots:1\n"),
            Stack::Resolver("9.9.9.9".parse().unwrap()),
        );

        let rendered = render_status(&status);
        assert!(
            rendered.contains("    Search: example.org local\n"),
            "{rendered}"
        );
        assert!(rendered.contains("   Options: ndots:1\n"), "{rendered}");
    }

    #[test]
    fn dns_lists_the_resolvers_and_where_they_are_from() {
        let status = sample(
            Some("nameserver 9.9.9.9\n"),
            Stack::Resolver("9.9.9.9".parse().unwrap()),
        );
        assert_eq!(
            render_dns(&status, false),
            "RESOLVER ORIGIN\n9.9.9.9  file\n"
        );

        // The stack says something else: the file is behind, and the listing
        // says which resolver the stack holds.
        let status = sample(
            Some("nameserver 9.9.9.9\n"),
            Stack::Resolver("1.1.1.1".parse().unwrap()),
        );
        let listing = render_dns(&status, false);
        assert!(listing.contains("9.9.9.9  file\n"), "{listing}");
        assert!(listing.contains("1.1.1.1  stack\n"), "{listing}");

        // The file says the same resolver twice: the file is not what the stack
        // says (one resolver), so the stack is named too — the next `update`
        // brings the file back to one line.
        let status = sample(
            Some("nameserver 9.9.9.9\nnameserver 9.9.9.9\n"),
            Stack::Resolver("9.9.9.9".parse().unwrap()),
        );
        assert!(!status.in_sync());
        let listing = render_dns(&status, false);
        assert!(listing.contains("9.9.9.9  file\n"), "{listing}");
        assert!(listing.contains("9.9.9.9  stack\n"), "{listing}");
    }

    #[test]
    fn dns_can_leave_the_header_out_and_says_when_there_is_nothing() {
        let status = sample(None, Stack::Empty);
        assert_eq!(render_dns(&status, true), "no resolver is configured\n");

        let status = sample(Some("nameserver 9.9.9.9\n"), Stack::Empty);
        assert_eq!(render_dns(&status, true), "9.9.9.9 file\n");
    }

    #[test]
    fn a_sync_says_what_was_written_or_that_there_was_nothing_to_do() {
        let sync = Sync {
            path: PathBuf::from("/etc/resolv.conf"),
            conf: parse("nameserver 9.9.9.9\n"),
            rendered: "nameserver 9.9.9.9\n".to_owned(),
            changed: true,
        };
        assert_eq!(
            render_sync(&sync, true),
            "would write /etc/resolv.conf:\nnameserver 9.9.9.9\n"
        );
        assert_eq!(
            render_sync(&sync, false),
            "wrote /etc/resolv.conf:\nnameserver 9.9.9.9\n"
        );

        let sync = Sync {
            changed: false,
            ..sync
        };
        assert_eq!(
            render_sync(&sync, false),
            "/etc/resolv.conf: already in sync\n"
        );
    }
}
