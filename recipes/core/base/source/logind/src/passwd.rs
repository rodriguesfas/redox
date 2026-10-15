//! Reading `/etc/passwd`.
//!
//! `utmp` records a login *name* but not its numeric user id, so `loginctl`
//! resolves the id here. Only the field layout is handled: three colon-separated
//! fields before the user id, and the rest ignored, which is all `loginctl`
//! needs and keeps the parser tolerant of the fields Redox does not use.

use std::fs;

/// The account database.
pub const PASSWD_PATH: &str = "/etc/passwd";

/// Looks a user id up in the contents of a `passwd` file.
///
/// Comments and malformed lines are skipped rather than rejected: a damaged
/// entry must not hide the rest of the database.
pub fn parse_uid(contents: &str, user: &str) -> Option<u32> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut fields = line.split(':');
        let name = fields.next()?;
        if name != user {
            continue;
        }

        // `name:password:uid:gid:...`
        let _password = fields.next()?;
        let uid = fields.next()?;
        if let Ok(uid) = uid.trim().parse::<u32>() {
            return Some(uid);
        }
    }
    None
}

/// Looks a user id up in `/etc/passwd`.
pub fn lookup_uid(user: &str) -> Option<u32> {
    let contents = fs::read_to_string(PASSWD_PATH).ok()?;
    parse_uid(&contents, user)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
# name:password:uid:gid:gecos:home:shell
root:x:0:0:root:/root:/bin/ion
user:x:1000:1000:User:/home/user:/bin/ion
daemon::1:1::/:/bin/false
# trailing comment
broken
nocolon
";

    #[test]
    fn users_are_found_by_name() {
        assert_eq!(parse_uid(PASSWD, "root"), Some(0));
        assert_eq!(parse_uid(PASSWD, "user"), Some(1000));
        assert_eq!(parse_uid(PASSWD, "daemon"), Some(1));
    }

    #[test]
    fn unknown_and_malformed_entries_are_ignored() {
        assert_eq!(parse_uid(PASSWD, "nobody"), None);
        assert_eq!(parse_uid(PASSWD, "broken"), None);
        assert_eq!(parse_uid(PASSWD, "nocolon"), None);
        assert_eq!(parse_uid("user:x:notanumber:0::/:/bin/sh\n", "user"), None);
        assert_eq!(parse_uid("", "root"), None);
        // A name that is a prefix of another must not match.
        assert_eq!(parse_uid("user2:x:1001:1001::/:/bin/sh\n", "user"), None);
    }

    #[test]
    fn missing_password_and_gecos_fields_are_fine() {
        assert_eq!(
            parse_uid("user::1000::/home/user:/bin/sh\n", "user"),
            Some(1000)
        );
        assert_eq!(parse_uid("user:x:1000\n", "user"), Some(1000));
    }

    #[test]
    fn lookup_on_the_host_returns_the_user_running_the_tests() {
        // The test process is logged in, so the lookup path itself is exercised.
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
        if let Some(uid) = lookup_uid(&user) {
            assert_eq!(
                parse_uid(&fs::read_to_string(PASSWD_PATH).unwrap(), &user),
                Some(uid)
            );
        }
    }
}
