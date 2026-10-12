//! Source-level guard for the aarch64 NVMe patch.
//!
//! ## Why this is a test and not a comment
//!
//! The first version of `nvmed-aarch64-poll-fence.patch` woke external-event
//! waiters by **draining** `executor::LocalExecutor::external_event`:
//!
//! ```ignore
//! let waiters: Vec<_> = self.external_event.borrow_mut().drain().collect();
//! for (_, (task, flags_ptr)) in waiters {
//!     unsafe { flags_ptr.as_ptr().write(EventFlags::READ) };
//!     enqueue::<Hw>(task);
//! }
//! ```
//!
//! That is a use-after-free. `flags_ptr` points *into* the caller's
//! `ExternalEventHandle`, and each entry may only be removed by that handle's
//! own `Drop`. Draining removed the entries out from under the still-live
//! handles, so `Drop`'s `remove` became a no-op: the map no longer knew about
//! the waiter, while the queued task still held a pointer into it. When the
//! handle was freed, the queued task wrote to recycled memory.
//!
//! The failure was silent and cross-process. The driver still booted, and
//! `cat` and `virtio-netd` died with a guard-page fault on roughly 4 of 6
//! identical invocations, with no disk I/O involved — the corrupted pages
//! belonged to whoever else had been handed them. A boot canary that only
//! checks "did it reach a login prompt" passes straight through this.
//!
//! These tests read the patch as text. They cannot prove runtime memory safety
//! — that needs the in-guest canary in `platform/scripts/test.sh`. What they do
//! is fail *instantly and locally* on the specific mistake that was made, so a
//! regression cannot be committed and discovered four hours later in CI.

use std::path::{Path, PathBuf};

/// Absolute path to the repository root, derived from this test file.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("security/ sits directly below the repo root")
        .to_path_buf()
}

fn patch_text() -> String {
    let path = repo_root().join("platform/patches/aarch64/base-nvmed-aarch64-poll-fence.patch");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Only lines the patch adds (`+`) or adds as context; `-` lines describe the
/// upstream bug and legitimately mention the code we no longer want.
fn added_lines(patch: &str) -> Vec<&str> {
    patch
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .collect()
}

/// The wakeup helper introduced for poll mode.
fn poll_timer_body(patch: &str) -> String {
    let start = patch
        .find("fn on_poll_timer")
        .unwrap_or_else(|| panic!("patch no longer contains on_poll_timer; update this test"));
    patch[start..]
        .lines()
        .take_while(|l| {
            !l.trim_start()
                .starts_with("pub fn has_outstanding_commands")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn poll_timer_wakeup_does_not_drain_the_waiter_map() {
    let body = poll_timer_body(&patch_text());

    assert!(
        !body.contains("drain()"),
        "on_poll_timer must not drain `external_event`. Draining removes each entry \
         before `Drop for ExternalEventHandle` can remove it, so the handle's own \
         removal becomes a no-op and the queued task keeps a dangling pointer into \
         freed memory. Iterate over keys and leave removal to `Drop`.\n\n\
         offending body:\n{body}"
    );
}

#[test]
fn poll_timer_does_not_unsubscribe_waiters() {
    let body = poll_timer_body(&patch_text());

    assert!(
        !body.contains("unsubscribe"),
        "on_poll_timer must not unsubscribe the waiters it wakes. Only \
         `Drop for ExternalEventHandle` may unsubscribe, otherwise a live handle \
         loses its subscription and stops receiving real events.\n\n\
         offending body:\n{body}"
    );
}

#[test]
fn wakeup_releases_the_borrow_before_writing_through_the_pointer() {
    let added = added_lines(&patch_text()).join("\n");

    // The write goes through a `NonNull<EventFlags>` that is only valid while the
    // owning handle is alive. It is read out of the map, the `RefCell` borrow is
    // released, and only then is the pointer written through.
    assert!(
        added.contains("borrow()\n") || added.contains(".borrow()\n"),
        "expected the waiter snapshot to read out of a scoped borrow"
    );
    assert!(
        added.contains("keys().copied()"),
        "the wakeup should snapshot `external_event` keys with `keys().copied()` and \
         look each entry up, so the map still owns every live handle."
    );
}

#[test]
fn is_full_wraps_instead_of_comparing_against_tail_plus_one() {
    let patch = patch_text();
    let added = added_lines(&patch).join("\n");

    // The upstream form `head == tail + 1` misses the wrap (tail == len-1, head == 0)
    // and lets `submit_unchecked` overwrite an entry the controller has not consumed.
    assert!(
        !added.contains("self.head == self.tail + 1"),
        "is_full must not use the non-wrapping comparison; it overwrites unconsumed \
         SQEs at the wrap point."
    );
    assert!(
        added.contains("& (self.data.len() as u16 - 1)"),
        "is_full should mask with the queue length to wrap the tail index."
    );
}

#[test]
fn doorbell_writes_are_release_ordered() {
    let added = added_lines(&patch_text()).join("\n");

    let kicks = added.matches("fence(Ordering::Release);").count();
    assert_eq!(
        kicks, 2,
        "both SQ and CQ doorbells need a Release fence so the entry is visible to \
         the device before the doorbell is rung."
    );
}

#[test]
fn completion_reads_are_acquire_ordered() {
    let added = added_lines(&patch_text()).join("\n");

    assert!(
        added.contains("fence(Ordering::Acquire);"),
        "reading a completion entry must be followed by an Acquire fence, otherwise \
         the payload may be observed before the status write lands."
    );
}

#[test]
fn poll_mode_is_documented_as_breaking_userspace_on_this_image() {
    // Not a code assertion: this records a hard-won fact that is otherwise only in
    // a comment, and that test.sh relies on.
    let test_sh = std::fs::read_to_string(repo_root().join("platform/scripts/test.sh"))
        .expect("read test.sh");
    let ci =
        std::fs::read_to_string(repo_root().join(".github/workflows/ci.yml")).expect("read ci");

    // Only real invocations count; the comment explaining why it is absent must
    // still be allowed to name the flag.
    let invokes_gic_v3 = ci
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .any(|l| l.contains("--gic-v3"));

    assert!(
        !invokes_gic_v3,
        "CI must not run the aarch64 canary with --gic-v3: under GICv3/ITS this image \
         faults userspace, so the milestones the job exists to check are unreachable."
    );
    assert!(
        test_sh.contains("known to break userspace"),
        "test.sh should warn that --gic-v3 breaks userspace, so nobody re-adds it as a \
         fix for an unrelated failure."
    );
}

#[test]
fn the_profile_ships_coreutils() {
    let toml = std::fs::read_to_string(repo_root().join("platform/config/aarch64/ai-edge.toml"))
        .expect("read ai-edge.toml");

    assert!(
        toml.contains("coreutils"),
        "ai-edge must include coreutils: the upstream server.toml include does not, \
         so without it the image ships no `cat`, and neither the canary nor a human on \
         a serial console can read /etc/ai-platform."
    );
}
