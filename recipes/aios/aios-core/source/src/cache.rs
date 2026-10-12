//! Model cache: discovery, install and removal of GGUF models.

use crate::checksum::sha256_hex;
use crate::error::Result;
use crate::gguf::GGUF_MAGIC;
use crate::model::ModelMeta;
use crate::registry::RegistryEntry;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Default cache location baked into the AIOS images (see config/*.toml,
/// `/var/lib/ai/models`) and reused on the host for development.
pub const DEFAULT_MODELS_DIR: &str = "/var/lib/ai/models";

/// Models dir used by the CLI: env `AIOS_MODELS_DIR` overrides the default
/// (used for host development and test isolation).
pub fn default_models_dir() -> String {
    std::env::var("AIOS_MODELS_DIR").unwrap_or_else(|_| DEFAULT_MODELS_DIR.to_string())
}

/// Recursively find `.gguf` files under `dir`.
pub fn find_models(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    walk(dir, &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("gguf"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Convenience wrapper: list installed models (metadata read from disk).
pub fn list_installed(dir: &Path) -> Result<Vec<ModelMeta>> {
    let mut out = Vec::new();
    for path in find_models(dir)? {
        match ModelMeta::from_path(&path) {
            Ok(m) => out.push(m),
            Err(e) => {
                // Broken file: surface it so users can clean up.
                out.push(ModelMeta {
                    name: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string(),
                    path: path.display().to_string(),
                    size_bytes: 0,
                    header: None,
                });
                let _ = e;
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Sensible install name for a source file: the source file stem, sanitized.
pub fn default_name_for(src: &Path) -> String {
    src.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string()
}

/// Copy `src` into `dir` as `name.gguf`, computing its SHA-256, and return a
/// registry entry ready to be added. Fails if the file does not parse as a
/// GGUF (no data is left behind on failure).
pub fn install_model(src: &Path, dir: &Path, name: &str) -> Result<RegistryEntry> {
    if name.is_empty() {
        return Err("install: empty model name".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("install: invalid name {name:?}").into());
    }
    // Validate GGUF before copying anything (stream: metadata can exceed
    // fixed buffers for real vocabularies).
    {
        let mut f = fs::File::open(src)?;
        crate::gguf::parse_header(&mut f)?;
    }

    fs::create_dir_all(dir)?;
    let dest = dir.join(format!("{name}.gguf"));

    let file = fs::File::open(src)?;
    let reader = BufReader::new(file);
    let sha256 = sha256_hex(reader)?;
    let size_bytes = fs::metadata(src)?.len();

    fs::copy(src, &dest)?;

    Ok(RegistryEntry {
        name: name.to_string(),
        path: dest.display().to_string(),
        sha256,
        size_bytes,
    })
}

/// Remove the file behind a registry entry if it lives inside `cache_dir`
/// (refuses to delete files outside the cache).
pub fn remove_model(entry: &RegistryEntry, cache_dir: &Path) -> Result<()> {
    let path = PathBuf::from(&entry.path);
    let canonical = path.canonicalize()?;
    let cache = cache_dir.canonicalize()?;
    if !canonical.starts_with(&cache) {
        return Err(format!(
            "remove: {} is outside cache {}; refusing to delete",
            path.display(),
            cache.display()
        )
        .into());
    }
    fs::remove_file(&canonical)?;
    Ok(())
}

/// Guess cache path for a name that is not registered: `<dir>/<name>.gguf`.
pub fn cache_path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.gguf"))
}

fn read_head(src: &Path) -> Result<Vec<u8>> {
    const HEAD_LEN: usize = 4096;
    let mut buf = vec![0u8; HEAD_LEN];
    let mut file = fs::File::open(src)?;
    use std::io::Read;
    let n = file.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Verify a file starts with the GGUF magic. Cheap pre-check before full parse.
pub fn is_gguf_file(src: &Path) -> bool {
    match read_head(src) {
        Ok(buf) if buf.len() >= 4 => {
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) == GGUF_MAGIC
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("aios-cache-test-{}-{n}", std::process::id()))
    }

    fn minimal_gguf_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GGUF");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // tensor count
        b.extend_from_slice(&0u64.to_le_bytes()); // metadata kv count
        b
    }

    #[test]
    fn install_roundtrip() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.gguf");
        fs::write(&src, minimal_gguf_bytes()).unwrap();

        let cache = dir.join("cache");
        let e = install_model(&src, &cache, "demo").unwrap();
        assert!(Path::new(&e.path).is_file());
        assert_eq!(e.size_bytes, 24);
        assert_eq!(e.sha256.len(), 64);
        assert!(is_gguf_file(Path::new(&e.path)));
        assert_eq!(find_models(&cache).unwrap().len(), 1);

        remove_model(&e, &cache).unwrap();
        assert!(!Path::new(&e.path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_rejects_bad_input() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.bin");
        fs::write(&bad, b"not a gguf").unwrap();

        let cache = dir.join("cache");
        assert!(install_model(&bad, &cache, "x").is_err());
        // Nothing left behind on failure.
        assert!(!cache.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_rejects_bad_name() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.gguf");
        fs::write(&src, minimal_gguf_bytes()).unwrap();
        let cache = dir.join("cache");
        assert!(install_model(&src, &cache, "").is_err());
        assert!(install_model(&src, &cache, "a/b").is_err());
        assert!(install_model(&src, &cache, r"a\b").is_err());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn remove_refuses_outside_cache() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let outside = dir.join("target.gguf");
        fs::write(&outside, minimal_gguf_bytes()).unwrap();
        let entry = RegistryEntry {
            name: "outside".into(),
            path: outside.display().to_string(),
            sha256: String::new(),
            size_bytes: 0,
        };
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).unwrap();
        assert!(remove_model(&entry, &cache).is_err());
        assert!(outside.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
