//! BCM2711 GENET — v35.
//!
//! Canal principal de diagnóstico: **salva no cartão o log que aparece no
//! monitor** (buffer do kernel em /scheme/sys/log), a cada etapa e
//! periodicamente. Também grava marcos separados por etapa.
//!
//! Etapas:
//!   0 daemon iniciou (userspace subiu)   -> boot-00.log
//!   1 vivo 10 s depois                   -> boot-01.log
//!   2 CPRMAN lido (seguro)               -> boot-02.log
//!   3 CPRMAN escrito (suspeito)          -> boot-03.log
//!   4 vivo 10 s após escrever            -> boot-04.log
//!   5 GENET lido                         -> boot-05.log
//!   6 sobreviveu ao GENET                -> boot-06.log
//!
//! Depois fica vivo e regrava boot-last.log quando o log cresce.

mod breadcrumb;
mod led;

use std::fs;
use std::io::Read;
use std::thread;
use std::time::Duration;

use common::{self, MemoryType, Prot};
use daemon::Daemon;

const CPRMAN_PHYS: usize = 0xfe1_01000;
const GPIO_PHYS: usize = 0xfe20_0000;
const GENET_PHYS: usize = 0xfd58_0000;

const PASSWORD: u32 = 0x5A00_0000;
const ENAB: u32 = 1 << 4;
const GATE: u32 = 1 << 6;
const BUSY: u32 = 1 << 7;
const SRC_MASK: u32 = 0xF;
const DIVI_MASK: u32 = 0xFFF << 12;
const SRC_PLLD_PER: u32 = 6;

fn rd(base: usize, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

fn wr(base: usize, off: usize, val: u32) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

/// Copia o log do kernel (o que está no monitor) para um arquivo no cartão.
fn dump_kernel_log(tag: &str) {
    let mut buf = Vec::new();
    if let Ok(mut f) = fs::File::open("/scheme/sys/log") {
        let _ = f.read_to_end(&mut buf);
    }
    if buf.is_empty() {
        return;
    }
    let path = format!("/var/lib/ai/boot-{}.log", tag);
    if fs::write(&path, &buf).is_ok() {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = f.sync_all();
        }
    }
}

fn mark(led: &Option<led::Led>, n: u32, path: &str, msg: &str) {
    breadcrumb::write_at(path, n, msg);
    dump_kernel_log(&format!("{:02}", n));
    println!("AIOS v35: etapa {} — {}", n, msg);
    if let Some(l) = led {
        l.stage((n + 1).min(6));
    }
}

fn main() {
    Daemon::new(daemon);
}

fn daemon(_: Daemon) -> ! {
    common::init();

    // Salva o log do boot IMEDIATAMENTE, antes de tocar em hardware.
    dump_kernel_log("00");
    println!("=== AIOS v35 (log no cartao) ===");

    let led = unsafe { common::physmap(GPIO_PHYS, 0x1000, Prot::RW, MemoryType::DeviceMemory) }
        .ok()
        .map(|p| {
            let l = led::Led::new(p as usize);
            l.init();
            l
        });

    // ---------- ETAPA 0 ----------
    mark(&led, 0, "/var/lib/ai/net-00", "daemon iniciou + log salvo");
    thread::sleep(Duration::from_secs(10));

    // ---------- ETAPA 1 ----------
    mark(&led, 1, "/var/lib/ai/net-01", "vivo apos 10s");
    thread::sleep(Duration::from_secs(2));

    // ---------- ETAPA 2: leitura CPRMAN ----------
    let mut info = String::new();
    let mut c250 = (0u32, 0u32);
    let mut c125 = (0u32, 0u32);
    let mut have_cpr = false;

    if let Ok(p) = unsafe { common::physmap(CPRMAN_PHYS, 0x2000, Prot::RW, MemoryType::DeviceMemory) }
    {
        let b = p as usize;
        have_cpr = true;
        c250 = (rd(b, 0x1e8), rd(b, 0x1ec));
        c125 = (rd(b, 0x210), rd(b, 0x214));
        info = format!(
            "GENET250={:08x}/{:08x} GENET125={:08x}/{:08x}",
            c250.0, c250.1, c125.0, c125.1
        );
    } else {
        info = "physmap CPRMAN falhou".to_string();
    }
    mark(&led, 2, "/var/lib/ai/net-02", &info);
    thread::sleep(Duration::from_secs(3));

    // ---------- ETAPA 3: escrita CPRMAN ----------
    let need250 = have_cpr && (c250.0 & ENAB == 0 || c250.0 & GATE == 0);
    let need125 = have_cpr && (c125.0 & ENAB == 0 || c125.0 & GATE == 0);
    let mut after;

    if have_cpr && (need250 || need125) {
        let b = unsafe {
            common::physmap(CPRMAN_PHYS, 0x2000, Prot::RW, MemoryType::DeviceMemory).unwrap() as usize
        };
        if need250 {
            if rd(b, 0x1ec) & DIVI_MASK == 0 {
                wr(b, 0x1ec, PASSWORD | (3u32 << 12));
            }
            let src = if rd(b, 0x1e8) & SRC_MASK == 0 {
                SRC_PLLD_PER
            } else {
                rd(b, 0x1e8) & SRC_MASK
            };
            wr(b, 0x1e8, PASSWORD | src | ENAB | GATE);
            for _ in 0..200 {
                if rd(b, 0x1e8) & BUSY == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        if need125 {
            if rd(b, 0x214) & DIVI_MASK == 0 {
                wr(b, 0x214, PASSWORD | (6u32 << 12));
            }
            let src = if rd(b, 0x210) & SRC_MASK == 0 {
                SRC_PLLD_PER
            } else {
                rd(b, 0x210) & SRC_MASK
            };
            wr(b, 0x210, PASSWORD | src | ENAB | GATE);
            for _ in 0..200 {
                if rd(b, 0x210) & BUSY == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        after = format!(
            "DEPOIS GENET250={:08x}/{:08x} GENET125={:08x}/{:08x}",
            rd(b, 0x1e8),
            rd(b, 0x1ec),
            rd(b, 0x210),
            rd(b, 0x214)
        );
    } else {
        after = "nada a ligar (ja tinham ENAB+GATE ou CPRMAN falhou)".to_string();
    }
    mark(&led, 3, "/var/lib/ai/net-03", &after);
    thread::sleep(Duration::from_secs(10));

    // ---------- ETAPA 4 ----------
    mark(&led, 4, "/var/lib/ai/net-04", "vivo 10s apos escrever CPRMAN");

    // ---------- ETAPA 5: GENET ----------
    match unsafe { common::physmap(GENET_PHYS, 0x1000, Prot::RW, MemoryType::DeviceMemory) } {
        Ok(p) => {
            let rev = unsafe { core::ptr::read_volatile(p as *const u32) };
            mark(
                &led,
                5,
                "/var/lib/ai/net-05",
                &format!("GENET SYS_REV={:08x}", rev),
            );
        }
        Err(e) => {
            mark(
                &led,
                5,
                "/var/lib/ai/net-05",
                &format!("physmap GENET falhou ({:?})", e),
            );
        }
    }
    thread::sleep(Duration::from_secs(5));

    // ---------- ETAPA 6 ----------
    mark(&led, 6, "/var/lib/ai/net-06", "sobreviveu ao GENET");

    // ---------- loop: mantém o log atualizado no cartão ----------
    let mut last_len = 0usize;
    let mut beat = 0u32;
    loop {
        thread::sleep(Duration::from_secs(15));
        beat += 1;
        let mut buf = Vec::new();
        if let Ok(mut f) = fs::File::open("/scheme/sys/log") {
            let _ = f.read_to_end(&mut buf);
        }
        if buf.len() != last_len {
            last_len = buf.len();
            let _ = fs::write("/var/lib/ai/boot-last.log", &buf);
            if let Ok(f) = fs::OpenOptions::new().write(true).open("/var/lib/ai/boot-last.log") {
                let _ = f.sync_all();
            }
        }
        breadcrumb::write_at(
            "/var/lib/ai/net-beat",
            beat,
            &format!("batimento {} (log {} bytes)", beat, buf.len()),
        );
        if let Some(l) = &led {
            l.success();
        }
    }
}
