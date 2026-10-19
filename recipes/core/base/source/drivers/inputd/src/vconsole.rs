//! The keyboard mapping of the console, in `/etc/vconsole.conf`.
//!
//! One line, the way the file is written: `KEYMAP=br-abnt2`. The mapping is
//! read where it is applied, and not by `inputd`: it starts from the initfs,
//! before `/etc` exists, so `localectl apply` reads the file after the root
//! filesystem is mounted.

use std::io;
use std::path::Path;

use crate::keymap::KeymapKind;

/// Where the mapping of the console is written.
pub const PATH: &str = "/etc/vconsole.conf";

/// The key of the file that holds the mapping.
const KEY: &str = "KEYMAP";

/// Reads the mapping of `text`, the way it is written.
///
/// Comments, blank lines and the keys of other programs are ignored, and a
/// file that says nothing — or a mapping that does not exist — is not an error:
/// a console without a mapping keeps the default layout.
pub fn parse(text: &str) -> Option<KeymapKind> {
    for line in text.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() != KEY {
            continue;
        }

        // Quotes are allowed, as in the other files of the console.
        return value.trim().trim_matches('"').parse::<KeymapKind>().ok();
    }

    None
}

/// The line a mapping is written as.
pub fn format(kind: KeymapKind) -> String {
    format!("{KEY}={kind}\n")
}

/// Reads the mapping of a file, if there is one.
pub fn load(path: &Path) -> Option<KeymapKind> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text)
}

/// Writes the mapping to a file.
pub fn save(path: &Path, kind: KeymapKind) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    std::fs::write(path, format(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_the_way_it_is_written() {
        for kind in KeymapKind::ALL {
            assert_eq!(parse(&format(kind)), Some(kind));
        }
    }

    #[test]
    fn the_other_lines_are_left_alone() {
        // Comments, blank lines and the keys of other programs.
        let text = "# the console\n\nLOCALE=pt_BR.UTF-8\nKEYMAP=br-abnt2\n";
        assert_eq!(parse(text), Some(KeymapKind::Abnt2));

        // The spaces and the quotes of the other files.
        assert_eq!(parse("KEYMAP = \"abnt2\"\n"), Some(KeymapKind::Abnt2));
    }

    #[test]
    fn a_file_that_says_nothing_has_no_mapping() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("LOCALE=pt_BR.UTF-8\n"), None);
        // A mapping that does not exist is not a mapping, and a line without a
        // value says nothing.
        assert_eq!(parse("KEYMAP=nordic\n"), None);
        assert_eq!(parse("KEYMAP\n"), None);
    }

    #[test]
    fn a_file_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("inputd-vconsole-{}", std::process::id()));
        let path = dir.join("vconsole.conf");

        assert_eq!(load(&path), None);

        save(&path, KeymapKind::Abnt2).unwrap();
        assert_eq!(load(&path), Some(KeymapKind::Abnt2));

        save(&path, KeymapKind::US).unwrap();
        assert_eq!(load(&path), Some(KeymapKind::US));

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
