#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::from_over_into)]
#![allow(clippy::module_inception)]

#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate log;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use yadict::parser;
use yadict::render::{DefaultRender, Render};

#[derive(Parser)]
#[command(name = "yadict", about = "MDict dictionary lookup tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Translate a word by querying all dictionaries in ~/.yadict/mdicts/
    Translate {
        /// Word to look up
        word: String,
    },

    /// Download a remote .mdx dictionary to ~/.yadict/mdicts/
    Add {
        /// HTTP(S) URL of the .mdx file to download
        url: String,
    },

    /// List all installed dictionaries in ~/.yadict/mdicts/
    List,
}

/// URL-keyed disk cache. Index is stored in `~/.yadict/cache.tsv` (tab-separated: url\tabsolute_path).
struct CacheStore {
    index_path: PathBuf,
}

impl CacheStore {
    fn new(cache_dir: &std::path::Path) -> Self {
        Self {
            index_path: cache_dir.join("cache.tsv"),
        }
    }

    /// Look up a cached local path by URL. Returns None if not found or the file has been deleted.
    fn lookup(&self, url: &str) -> Option<PathBuf> {
        let content = std::fs::read_to_string(&self.index_path).ok()?;
        content.lines().find_map(|line| {
            let (stored_url, stored_path) = line.split_once('\t')?;
            if stored_url == url {
                let path = PathBuf::from(stored_path);
                path.exists().then_some(path)
            } else {
                None
            }
        })
    }

    /// Append a URL -> local path mapping to the index.
    fn insert(&self, url: &str, path: &std::path::Path) -> anyhow::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.index_path)?;
        writeln!(f, "{}\t{}", url, path.display())?;
        Ok(())
    }
}

/// Download a remote dictionary URL to ~/.yadict/mdicts/ and record it in the cache index.
/// Returns the local path. If the URL is already cached and the file exists, returns
/// the cached path immediately without re-downloading.
fn download_to_cache(url: &str) -> anyhow::Result<PathBuf> {
    let cache_dir = yadict_home()?.join("mdicts");
    std::fs::create_dir_all(&cache_dir)?;

    let store = CacheStore::new(&cache_dir);

    if let Some(cached) = store.lookup(url) {
        debug!("using cached file: {}", cached.display());
        return Ok(cached);
    }

    eprintln!("Downloading {} ...", url);
    let resp = ureq::get(url)
        .call()
        .map_err(|e| anyhow!("Download failed: {e}"))?;

    // Resolve filename: Content-Disposition header takes priority over URL path.
    let filename = resolve_filename(url, &resp)?;
    let dest = cache_dir.join(&filename);

    let mut file = std::fs::File::create(&dest)?;
    std::io::copy(&mut resp.into_reader(), &mut file)?;

    store.insert(url, &dest)?;
    eprintln!("Saved to {}", dest.display());

    Ok(dest)
}

/// Determine the local filename for a downloaded resource.
/// Prefers the `Content-Disposition` header; falls back to the percent-decoded URL path segment.
fn resolve_filename(url: &str, resp: &ureq::Response) -> anyhow::Result<String> {
    if let Some(cd) = resp.header("content-disposition") {
        if let Some(name) = parse_content_disposition(cd) {
            return Ok(name);
        }
    }

    let raw = url
        .split('/')
        .last()
        .and_then(|s| s.split('?').next())
        .and_then(|s| s.split('#').next())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Cannot derive filename from URL: {}", url))?;

    Ok(percent_decode(raw))
}

/// Parse the filename from a `Content-Disposition` header value.
/// Handles both `filename="foo.mdx"` and the RFC 5987 `filename*=UTF-8''foo.mdx` form.
fn parse_content_disposition(header: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    for part in header.split(';') {
        let part = part.trim();
        // RFC 5987 extended form takes priority (filename*=charset'lang'encoded).
        if let Some(val) = part.strip_prefix("filename*=") {
            let encoded = val.splitn(3, '\'').nth(2).unwrap_or(val);
            return Some(percent_decode(encoded));
        }
        if let Some(val) = part.strip_prefix("filename=") {
            let name = val.trim().trim_matches('"');
            if !name.is_empty() {
                plain = Some(name.to_string());
            }
        }
    }
    plain
}

/// Percent-decode a URL-encoded string, returning valid UTF-8.
fn percent_decode(s: &str) -> String {
    let src = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%' && i + 2 < src.len() {
            if let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&src[i + 1..i + 3]).unwrap_or(""), 16)
            {
                buf.push(b);
                i += 3;
                continue;
            }
        }
        buf.push(src[i]);
        i += 1;
    }
    String::from_utf8(buf).unwrap_or_else(|_| s.to_string())
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow!("HOME environment variable is not set"))
}

/// Returns the yadict data directory: $YADICT_HOME if set, otherwise ~/.yadict.
fn yadict_home() -> anyhow::Result<PathBuf> {
    if let Ok(val) = std::env::var("YADICT_HOME") {
        return Ok(PathBuf::from(val));
    }
    Ok(home_dir()?.join(".yadict"))
}

fn main() -> anyhow::Result<()> {
    env_logger::try_init().ok();

    let default_render = DefaultRender::default();

    let cli = Cli::parse();

    match cli.command {
        Commands::Add { url } => {
            download_to_cache(&url)?;
        }
        Commands::List => {
            let mdicts_dir = yadict_home()?.join("mdicts");
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&mdicts_dir)
                .map_err(|e| anyhow!("Cannot read {}: {e}", mdicts_dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map_or(false, |ext| ext == "mdx"))
                .collect();
            paths.sort();
            if paths.is_empty() {
                println!("No dictionaries installed. Use 'yadict add <URL>' to install one.");
            } else {
                for path in &paths {
                    println!("{}", path.file_name().unwrap_or_default().to_string_lossy());
                }
            }
        }
        Commands::Translate { word } => {
            let mdicts_dir = yadict_home()?.join("mdicts");

            let mut paths: Vec<PathBuf> = std::fs::read_dir(&mdicts_dir)
                .map_err(|e| anyhow!("Cannot read {}: {e}", mdicts_dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map_or(false, |ext| ext == "mdx"))
                .collect();
            paths.sort();

            if paths.is_empty() {
                eprintln!(
                    "No dictionaries found in {}. Use 'yadict add <URL>' to install one.",
                    mdicts_dir.display()
                );
                return Ok(());
            }

            let divider = "\x1b[38;5;240m".to_string() + &"─".repeat(60) + "\x1b[0m";
            let mut results: Vec<String> = Vec::new();

            for path in &paths {
                let mdx = match parser::parse(path) {
                    Ok(m) => m,
                    Err(e) => {
                        debug!("skipping {}: {e}", path.display());
                        continue;
                    }
                };
                for record in mdx.lookup(&word) {
                    if let Some(bytes) = record.value() {
                        let value = String::from_utf8_lossy(bytes);
                        results.push(default_render.render(value)?);
                    }
                }
            }

            if results.is_empty() {
                println!("'{}' not found in any dictionary", word);
            } else {
                println!("{}", results.join(&format!("\n{}\n", divider)));
            }
        }
    }

    Ok(())
}
