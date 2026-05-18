use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Public data types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    /// Display name of the dictionary.
    pub name: String,
    /// Direct download URL of the `.mdx` file.
    pub url: String,
    /// File size in bytes (0 = unknown).
    pub size: u64,
    /// Independent category paths this entry belongs to.
    /// Each element may contain " / " to denote sub-hierarchy within that category.
    pub categories: Vec<String>,
    /// Name of the registry this entry came from.
    pub registry: String,
}

impl DictEntry {
    /// Returns a compact human-readable file-size string, or "?" when unknown.
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
    fn entries(&self) -> &[DictEntry];
    fn search(&self, query: &str) -> Vec<&DictEntry>;
    /// Reload all entries from installed registry files.
    fn refresh(&mut self) -> Result<()>;
    /// True when no registries are installed.
    fn is_stale(&self) -> bool;
}

// ── YAML file schema ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct YamlMeta {
    name: String,
    #[allow(dead_code)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YamlResource {
    name: String,
    #[serde(default)]
    category: Vec<String>,
    mdx: String,
}

#[derive(Debug, Deserialize)]
struct YamlFile {
    metadata: YamlMeta,
    resources: Vec<YamlResource>,
}

// ── Built-in registry ─────────────────────────────────────────────────────────

const BUILTIN_REGISTRY: &str = include_str!("../../registry.yaml");

// ── LocalRegistry ─────────────────────────────────────────────────────────────

/// Reads all `.yaml` / `.yml` registry files from `~/.yadict/registries/`.
/// The built-in registry (registry.yaml) is always loaded first.
pub struct LocalRegistry {
    entries: Vec<DictEntry>,
    dir: PathBuf,
}

impl LocalRegistry {
    /// Load the built-in registry plus all user-installed registries from `<home>/registries/`.
    pub fn new(home_dir: &Path) -> Self {
        let dir = home_dir.join("registries");
        let mut entries = Self::parse_yaml(BUILTIN_REGISTRY).unwrap_or_default();
        entries.extend(Self::load_dir(&dir).unwrap_or_default());
        Self { entries, dir }
    }

    /// Install a registry YAML from a local file path or HTTP(S) URL.
    /// The file is saved to `<home>/registries/<registry_name>.yaml`.
    /// Installing a registry with an existing name overwrites the previous file.
    pub fn install(src: &str, home_dir: &Path) -> Result<()> {
        let content = if src.starts_with("https://") || src.starts_with("http://") {
            eprintln!("Downloading registry from {} ...", src);
            reqwest::blocking::get(src)
                .and_then(|r| r.error_for_status())
                .map_err(|e| anyhow!("Download failed: {e}"))?
                .text()?
        } else {
            std::fs::read_to_string(src)?
        };

        let yaml: YamlFile = serde_yaml::from_str(&content)
            .map_err(|e| anyhow!("Invalid registry YAML: {e}"))?;

        let dir = home_dir.join("registries");
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{}.yaml", yaml.metadata.name));
        std::fs::write(&dest, &content)?;

        eprintln!(
            "Registry '{}' installed ({} dicts) → {}",
            yaml.metadata.name,
            yaml.resources.len(),
            dest.display()
        );
        Ok(())
    }

    fn load_dir(dir: &Path) -> Result<Vec<DictEntry>> {
        let mut entries = Vec::new();
        if !dir.exists() {
            return Ok(entries);
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
            })
            .collect();
        paths.sort();
        for path in paths {
            match Self::load_file(&path) {
                Ok(e) => entries.extend(e),
                Err(err) => eprintln!("Warning: skipping {}: {err}", path.display()),
            }
        }
        Ok(entries)
    }

    fn load_file(path: &Path) -> Result<Vec<DictEntry>> {
        let content = std::fs::read_to_string(path)?;
        Self::parse_yaml(&content)
            .map_err(|e| anyhow!("{} in {}", e, path.display()))
    }

    fn parse_yaml(content: &str) -> Result<Vec<DictEntry>> {
        let yaml: YamlFile =
            serde_yaml::from_str(content).map_err(|e| anyhow!("Parse error: {e}"))?;
        let registry = yaml.metadata.name.clone();
        Ok(yaml
            .resources
            .into_iter()
            .map(|r| DictEntry {
                name: r.name,
                url: r.mdx,
                size: 0,
                categories: r.category,
                registry: registry.clone(),
            })
            .collect())
    }
}

impl Registry for LocalRegistry {
    fn entries(&self) -> &[DictEntry] {
        &self.entries
    }

    fn search(&self, query: &str) -> Vec<&DictEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.categories.iter().any(|c| c.to_lowercase().contains(&q))
                    || e.registry.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn refresh(&mut self) -> Result<()> {
        let mut entries = Self::parse_yaml(BUILTIN_REGISTRY).unwrap_or_default();
        entries.extend(Self::load_dir(&self.dir).unwrap_or_default());
        self.entries = entries;
        eprintln!("Reloaded {} entries from registries.", self.entries.len());
        Ok(())
    }

    fn is_stale(&self) -> bool {
        false
    }
}
