//! GGUF model file header/metadata parser (pure Rust, no external deps).
//!
//! Format (little-endian):
//!   magic        u32    = 0x46554747 ("GGUF")
//!   version      u32
//!   tensor_count u64
//!   metadata_kv_count u64
//!   metadata: repeated (key, value_type u32, value)

use std::io::Read;

use crate::error::{Error, Result};

pub const GGUF_MAGIC: u32 = 0x4655_4747;

/// Upper bound on the metadata key/value count, and on any single string or
/// byte-array length, declared by a GGUF file.
///
/// These counts are attacker-controlled: they come straight out of the file,
/// and the parser turns them into allocations. Without a bound, a four-byte
/// `kv_count` of `u64::MAX` makes `Vec::with_capacity` panic with "capacity
/// overflow", and a plausible-looking length makes the parser try to allocate
/// gigabytes before `read_exact` discovers the file is truncated. Both are
/// panics or OOM on a bad file rather than a parse error, and this parser
/// runs on files that arrive over the network during `aios-deploy`, so a
/// corrupt or hostile model must produce an `Err`, never a crash.
///
/// The limits are far above any real model: metadata entries are per-key (a
/// 32k-vocab tokenizer is a handful of array-valued entries, not 32k of
/// them), and the largest single real metadata string is a tokenizer vocab
/// list well under a megabyte.
pub const MAX_METADATA_ENTRIES: u64 = 1_000_000;
pub const MAX_STRING_LEN: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ValueType {
    Uint8 = 0,
    Int8 = 1,
    Uint16 = 2,
    Int16 = 3,
    Uint32 = 4,
    Int32 = 5,
    Float32 = 6,
    Bool = 7,
    String = 8,
    Array = 9,
    Uint64 = 10,
    Int64 = 11,
    Float64 = 12,
    Unknown(u32),
}

impl From<u32> for ValueType {
    fn from(v: u32) -> Self {
        match v {
            0 => ValueType::Uint8,
            1 => ValueType::Int8,
            2 => ValueType::Uint16,
            3 => ValueType::Int16,
            4 => ValueType::Uint32,
            5 => ValueType::Int32,
            6 => ValueType::Float32,
            7 => ValueType::Bool,
            8 => ValueType::String,
            9 => ValueType::Array,
            10 => ValueType::Uint64,
            11 => ValueType::Int64,
            12 => ValueType::Float64,
            other => ValueType::Unknown(other),
        }
    }
}

impl ValueType {
    pub fn name(&self) -> String {
        format!("{self:?}")
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Uint8(u8),
    Int8(i8),
    Uint16(u16),
    Int16(i16),
    Uint32(u32),
    Int32(i32),
    Float32(f32),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Uint64(u64),
    Int64(i64),
    Float64(f64),
    Unknown(u32, Vec<u8>),
}

impl Value {
    /// Compact human-readable form, e.g. for `ai inspect`.
    pub fn brief(&self) -> String {
        match self {
            Value::String(s) => format!("\"{s}\""),
            Value::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .take(8)
                    .map(|v| v.brief())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::Bool(b) => b.to_string(),
            Value::Unknown(t, bytes) => format!("<unknown type {t}: {} bytes>", bytes.len()),
            other => format!("{other:?}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub key: String,
    pub value_type: ValueType,
    pub value: Value,
}

/// Header of a GGUF file plus its parsed metadata entries.
#[derive(Debug, Clone)]
pub struct GgufHeader {
    pub version: u32,
    pub tensor_count: u64,
    pub metadata: Vec<Metadata>,
}

impl GgufHeader {
    /// Find a metadata entry by key (first match).
    pub fn get(&self, key: &str) -> Option<&Metadata> {
        self.metadata.iter().find(|m| m.key == key)
    }

    /// Read a string metadata value by key (e.g. `general.architecture`).
    pub fn string(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Metadata {
                value: Value::String(s),
                ..
            }) => Some(s.clone()),
            _ => None,
        }
    }

    /// Convenience getters used by the model registry.
    pub fn general_name(&self) -> Option<String> {
        match self.get("general.name") {
            Some(Metadata {
                value: Value::String(s),
                ..
            }) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn general_file_type(&self) -> Option<u32> {
        match self.get("general.file_type") {
            Some(Metadata {
                value: Value::Uint32(v),
                ..
            }) => Some(*v),
            Some(Metadata {
                value: Value::Int32(v),
                ..
            }) => Some(*v as u32),
            _ => None,
        }
    }
}

/// Read and parse the GGUF header by streaming from `src`. Metadata regions of
/// real models (tokenizer vocabularies) can exceed any fixed buffer, so this
/// reads incrementally until the metadata block ends.
pub fn parse_header<R: Read>(src: &mut R) -> Result<GgufHeader> {
    let mut pos = 0u64;

    let magic = read_u32(src, &mut pos)?;
    if magic != GGUF_MAGIC {
        return Err(Error::Msg(format!(
            "invalid GGUF magic 0x{magic:08x} (expected 0x{GGUF_MAGIC:08x})"
        )));
    }
    let version = read_u32(src, &mut pos)?;
    let tensor_count = read_u64(src, &mut pos)?;
    let kv_count = read_u64(src, &mut pos)?;

    // Bound the pre-allocation. Without this, `with_capacity` panics with
    // "capacity overflow" on a crafted kv_count, aborting the whole process
    // instead of reporting a bad file. See MAX_METADATA_ENTRIES.
    if kv_count > MAX_METADATA_ENTRIES {
        return Err(Error::Msg(format!(
            "GGUF declares {kv_count} metadata entries, over the {MAX_METADATA_ENTRIES} limit"
        )));
    }
    let mut metadata = Vec::with_capacity(kv_count as usize);
    for _ in 0..kv_count {
        let key = read_string(src, &mut pos)?;
        let value_type = ValueType::from(read_u32(src, &mut pos)?);
        let value = read_value(src, &mut pos, value_type)?;
        metadata.push(Metadata {
            key,
            value_type,
            value,
        });
    }

    Ok(GgufHeader {
        version,
        tensor_count,
        metadata,
    })
}

/// Convenience for callers that already hold the bytes in memory.
pub fn parse_header_buf(buf: &[u8]) -> Result<GgufHeader> {
    let mut slice = buf;
    parse_header(&mut slice)
}

/// Read `len` bytes, refusing sizes we will not honour.
///
/// `len` originates in the file and is therefore attacker-controlled, so both
/// checks matter and neither is optional:
///   * `MAX_STRING_LEN` rejects absurd lengths that would otherwise turn into an
///     OOM or a "capacity overflow" panic instead of a parse error.
///   * `usize::try_from` rejects lengths that do not fit a `usize`. Skipping it
///     is a real truncation bug on 32-bit targets: a `u64` length above
///     `usize::MAX` narrows to a small value, so we would allocate a small
///     buffer and then read the wrong number of bytes, silently mis-parsing
///     every following field.
fn read_bytes<R: Read>(src: &mut R, pos: &mut u64, len: u64) -> Result<Vec<u8>> {
    if len > MAX_STRING_LEN {
        return Err(Error::Msg(format!(
            "GGUF declares a {len}-byte field at offset {pos}, over the {} byte limit",
            MAX_STRING_LEN
        )));
    }
    let len = usize::try_from(len).map_err(|_| {
        Error::Msg(format!(
            "GGUF declares a {len}-byte field at offset {pos}, which does not fit this target's \
             address space"
        ))
    })?;
    let mut v = vec![0u8; len];
    src.read_exact(&mut v)
        .map_err(|_| Error::Msg(format!("truncated GGUF (need {len} bytes at offset {pos})")))?;
    *pos += len as u64;
    Ok(v)
}

fn read_u32<R: Read>(src: &mut R, pos: &mut u64) -> Result<u32> {
    let s = read_bytes(src, pos, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64<R: Read>(src: &mut R, pos: &mut u64) -> Result<u64> {
    let s = read_bytes(src, pos, 8)?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn read_string<R: Read>(src: &mut R, pos: &mut u64) -> Result<String> {
    // Pass the length through as u64 so read_bytes does the usize conversion
    // under its own checks, rather than truncating here.
    let len = read_u64(src, pos)?;
    let s = read_bytes(src, pos, len)?;
    String::from_utf8(s).map_err(|e| Error::Msg(format!("invalid utf-8 in GGUF string: {e}")))
}

fn read_value<R: Read>(src: &mut R, pos: &mut u64, vt: ValueType) -> Result<Value> {
    Ok(match vt {
        ValueType::Uint8 => Value::Uint8(read_bytes(src, pos, 1)?[0]),
        ValueType::Int8 => Value::Int8(read_bytes(src, pos, 1)?[0] as i8),
        ValueType::Uint16 => {
            let s = read_bytes(src, pos, 2)?;
            Value::Uint16(u16::from_le_bytes([s[0], s[1]]))
        }
        ValueType::Int16 => {
            let s = read_bytes(src, pos, 2)?;
            Value::Int16(i16::from_le_bytes([s[0], s[1]]))
        }
        ValueType::Uint32 => Value::Uint32(read_u32(src, pos)?),
        ValueType::Int32 => Value::Int32(read_u32(src, pos)? as i32),
        ValueType::Float32 => Value::Float32(f32::from_bits(read_u32(src, pos)?)),
        ValueType::Bool => Value::Bool(read_bytes(src, pos, 1)?[0] != 0),
        ValueType::String => Value::String(read_string(src, pos)?),
        ValueType::Array => {
            let elem_type = ValueType::from(read_u32(src, pos)?);
            let count = read_u64(src, pos)?;
            // Same reasoning as kv_count: a crafted count must not panic the
            // pre-allocation. Arrays are also nested-capable, so this bound is
            // per-array rather than for the whole file.
            if count > MAX_METADATA_ENTRIES {
                return Err(Error::Msg(format!(
                    "GGUF declares an array of {count} elements, over the \
                     {MAX_METADATA_ENTRIES} limit"
                )));
            }
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                items.push(read_value(src, pos, elem_type)?);
            }
            Value::Array(items)
        }
        ValueType::Uint64 => Value::Uint64(read_u64(src, pos)?),
        ValueType::Int64 => Value::Int64(read_u64(src, pos)? as i64),
        ValueType::Float64 => Value::Float64(f64::from_bits(read_u64(src, pos)?)),
        ValueType::Unknown(t, ..) => {
            // Unknown type: skip its body conservatively (16 bytes). GGUF is
            // versioned; newer types keep fixed-size scalars.
            let bytes = read_bytes(src, pos, 16)?;
            Value::Unknown(t, bytes)
        }
    })
}
