//! Local model registry: a tiny catalog of installed models keyed by name,
//! recording path, sha256 and size. Stored as a TSV file (no external deps,
//! trivially readable on Redox and on the host).
//!
//! Format (`\t`-separated):
//!   name<TAB>path<TAB>sha256<TAB>size_bytes

use crate::error::Result;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_REGISTRY_FILE: &str = "/var/lib/ai/registry.tsv";

/// Registry file used by the CLI: env `AIOS_REGISTRY` overrides the default
/// (host development / test isolation).
pub fn default_registry_file() -> String {
    std::env::var("AIOS_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY_FILE.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

pub struct Registry {
    file: PathBuf,
    entries: Vec<RegistryEntry>,
}

impl Registry {
    pub fn load(file: impl Into<PathBuf>) -> Result<Self> {
        let file = file.into();
        let mut entries = Vec::new();
        if file.exists() {
            let text = fs::read_to_string(&file)?;
            for (i, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() != 4 {
                    return Err(format!(
                        "registry {} line {}: expected 4 cols",
                        file.display(),
                        i + 1
                    )
                    .into());
                }
                entries.push(RegistryEntry {
                    name: cols[0].to_string(),
                    path: cols[1].to_string(),
                    sha256: cols[2].to_string(),
                    size_bytes: cols[3].parse().map_err(|_| {
                        format!("registry {} line {}: bad size", file.display(), i + 1)
                    })?,
                });
            }
        }
        Ok(Registry { file, entries })
    }

    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    pub fn find(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn add(&mut self, entry: RegistryEntry) {
        if let Some(i) = self.entries.iter().position(|e| e.name == entry.name) {
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.name != name);
        self.entries.len() != before
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = String::from("# AIOS model registry: name\tpath\tsha256\tsize\n");
        for e in &self.entries {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                e.name, e.path, e.sha256, e.size_bytes
            ));
        }
        fs::write(&self.file, out)?;
        Ok(())
    }
}
