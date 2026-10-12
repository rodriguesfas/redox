//! Model metadata + helpers to describe a model on disk.

use crate::error::Result;
use crate::gguf::{self, GgufHeader};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ModelMeta {
    /// Canonical name: file stem, or `general.name` when present.
    pub name: String,
    /// Path to the GGUF file.
    pub path: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// GGUF header (parsed on demand; `None` if the file is not GGUF).
    pub header: Option<GgufHeader>,
}

impl ModelMeta {
    /// Read metadata from a GGUF file without loading the whole file: the
    /// header is read from the first ~KiB (metadata usually fits there).
    pub fn from_path(path: &Path) -> Result<Self> {
        let meta = fs::metadata(path)?;
        let size_bytes = meta.len();

        let name = file_stem(path).unwrap_or_else(|| path.display().to_string());

        let header = read_header_head(path)?;

        Ok(ModelMeta {
            name,
            path: path.display().to_string(),
            size_bytes,
            header,
        })
    }

    pub fn is_gguf(&self) -> bool {
        self.header.is_some()
    }

    /// Pretty size, e.g. "930.4 MiB".
    pub fn size_pretty(&self) -> String {
        pretty_bytes(self.size_bytes)
    }
}

/// Read and parse the GGUF header by streaming from the file. The metadata
/// region of real models (tokenizer vocabularies) can exceed small fixed
/// buffers, so we stream until the metadata block ends.
fn read_header_head(path: &Path) -> Result<Option<GgufHeader>> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() || u32::from_le_bytes(magic) != gguf::GGUF_MAGIC {
        return Ok(None);
    }
    let mut stream = magic.as_slice().chain(f);
    gguf::parse_header(&mut stream).map(Some)
}

pub fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

pub fn pretty_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KIB * KIB * KIB {
        format!("{:.1} GiB", b / (KIB * KIB * KIB))
    } else if b >= KIB * KIB {
        format!("{:.1} MiB", b / (KIB * KIB))
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{b} B")
    }
}

#[cfg(test)]
mod tests {
    use super::super::gguf;
    use super::*;

    /// Build a GGUF v3 buffer whose metadata block exceeds 1 MiB (a big
    /// tokenizer vocabulary). Regression test: headers this large must stream.
    fn big_gguf(kv_payloads: usize) -> Vec<u8> {
        let mut v = Vec::new();
        let mut push = |b: &[u8]| v.extend_from_slice(b);
        push(&0x4655_4747u32.to_le_bytes()); // magic
        push(&3u32.to_le_bytes()); // version
        push(&0u64.to_le_bytes()); // tensor count
        push(&1u64.to_le_bytes()); // kv count
        let key = "tokenizer.ggml.tokens";
        push(&(key.len() as u64).to_le_bytes());
        push(key.as_bytes());
        push(&9u32.to_le_bytes()); // array
        push(&8u32.to_le_bytes()); // array element type: string
        push(&(kv_payloads as u64).to_le_bytes());
        for i in 0..kv_payloads {
            let s = format!("tok{i:06}_abcdefghij"); // 16 bytes each
            push(&(s.len() as u64).to_le_bytes());
            push(s.as_bytes());
        }
        v
    }

    #[test]
    fn streams_metadata_larger_than_fixed_buffer() {
        let buf = big_gguf(80_000); // ~1.4 MiB of metadata
        assert!(buf.len() > 1 << 20, "test needs >1 MiB, got {}", buf.len());

        let mut slice = &buf[..];
        let h = gguf::parse_header(&mut slice).expect("streaming parse of big header");
        assert_eq!(h.version, 3);
        assert_eq!(h.metadata.len(), 1);
        let ml = &h.metadata[0];
        let gguf::Value::Array(items) = &ml.value else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 80_000);
    }

    #[test]
    fn sizes() {
        assert_eq!(pretty_bytes(0), "0 B");
        assert_eq!(pretty_bytes(1024), "1.0 KiB");
        assert_eq!(pretty_bytes(953_675_776), "909.5 MiB");
        assert_eq!(pretty_bytes((1024.0 * 1024.0 * 1024.0) as u64), "1.0 GiB");
    }
}
