mod crawler;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const BASE_URL: &str = "https://mdx.mdict.org";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    /// Display name: decoded filename without the `.mdx` extension.
    pub name: String,
    /// Direct download URL (URL-encoded, ready to pass to ureq or `yadict add`).
    pub url: String,
    /// File size in bytes.
    pub size: u64,
    /// Human-readable category derived from the directory path (decoded).
    pub category: String,
}

impl DictEntry {
    /// Returns a compact human-readable file-size string (KiB / MiB / GiB).
    pub fn size_human(&self) -> String {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * KIB;
        const GIB: u64 = 1024 * MIB;
        match self.size {
            0 => "?".to_string(),
            s if s >= GIB => format!("{:.1} GiB", s as f64 / GIB as f64),
            s if s >= MIB => format!("{:.1} MiB", s as f64 / MIB as f64),
            s => format!("{:.0} KiB", s as f64 / KIB as f64),
        }
    }
}

// ── Trait ─────────────────────────────────────────────────────────────────────

pub trait Registry {
    /// All indexed entries.
    fn entries(&self) -> &[DictEntry];

    /// Case-insensitive substring search over name and category.
    fn search(&self, query: &str) -> Vec<&DictEntry>;

    /// Re-crawl the remote index and persist the result.
    fn refresh(&mut self) -> Result<()>;

    /// True when the local cache is empty (never fetched or cleared).
    fn is_stale(&self) -> bool;
}

// ── Default implementation: mdx.mdict.org ────────────────────────────────────

pub struct MdictOrgRegistry {
    entries: Vec<DictEntry>,
    cache_path: PathBuf,
}

impl MdictOrgRegistry {
    /// Load from cache if available; otherwise start empty (call `refresh`).
    pub fn new(home_dir: &std::path::Path) -> Self {
        let cache_path = home_dir.join("registry.json");
        let entries = Self::load_cache(&cache_path).unwrap_or_default();
        Self { entries, cache_path }
    }

    fn load_cache(path: &std::path::Path) -> Option<Vec<DictEntry>> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save_cache(&self) -> Result<()> {
        if let Some(parent) = self.cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.cache_path, serde_json::to_string(&self.entries)?)?;
        Ok(())
    }
}

impl Registry for MdictOrgRegistry {
    fn entries(&self) -> &[DictEntry] {
        &self.entries
    }

    fn search(&self, query: &str) -> Vec<&DictEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.category.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn refresh(&mut self) -> Result<()> {
        eprintln!("Fetching dictionary index from {} ...", BASE_URL);
        let mut entries = crawler::crawl(BASE_URL)?;

        // Deduplicate by URL (the site may expose the same file in multiple paths).
        entries.sort_by(|a, b| a.url.cmp(&b.url));
        entries.dedup_by(|a, b| a.url == b.url);

        self.entries = entries;
        eprintln!("Indexed {} dictionaries.", self.entries.len());
        self.save_cache()
    }

    fn is_stale(&self) -> bool {
        self.entries.is_empty()
    }
}
