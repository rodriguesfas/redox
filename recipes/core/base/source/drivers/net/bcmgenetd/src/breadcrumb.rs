//! On-disk stage breadcrumb. Written to a persistent path on the root
//! filesystem so that, after a failed boot attempt, the SD card can be
//! brought back to the PC and the last reached stage read (via QEMU boot or
//! a future RedoxFS reader).

use std::fs;
use std::io::Write;

pub fn write(stage: u32, msg: &str, err: Option<&str>) {
    let mut content = format!("stage={}\nmsg={}\n", stage, msg);
    if let Some(e) = err {
        content.push_str(&format!("error={}\n", e));
    }
    let _ = fs::create_dir_all("/var/lib/ai");
    let path = "/var/lib/ai/net-stage";
    let _ = fs::write(path, &content);
    if let Ok(mut f) = fs::OpenOptions::new().write(true).open(path) {
        let _ = f.sync_all();
    }
}