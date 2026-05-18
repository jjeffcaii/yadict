use super::DictEntry;
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

// Match every <tr class="file">…</tr> block in a directory listing page.
static ROW_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<tr class="file">(.*?)</tr>"#).unwrap());
// Match the href inside a file/dir row. Only relative links (starting with "./").
static HREF_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(\./[^"?#]+)""#).unwrap());
// Match the data-order size attribute (-1 for directories, positive bytes for files).
static SIZE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-order="(-?\d+)""#).unwrap());

const MAX_DEPTH: usize = 10;
// Polite crawl delay between directory requests (ms).
const CRAWL_DELAY_MS: u64 = 120;

pub fn crawl(base_url: &str) -> Result<Vec<DictEntry>> {
    let mut entries = Vec::new();
    crawl_dir(base_url, "/", &mut entries, 0)?;
    Ok(entries)
}

fn crawl_dir(base_url: &str, path: &str, entries: &mut Vec<DictEntry>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }

    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let html = match ureq::get(&url).call() {
        Ok(resp) => resp.into_string()?,
        Err(e) => {
            eprintln!("  [warn] {}: {}", url, e);
            return Ok(());
        }
    };

    let category = path_to_category(path);
    let mut subdirs: Vec<String> = Vec::new();

    for cap in ROW_RE.captures_iter(&html) {
        let row = &cap[1];

        let href = match HREF_RE.captures(row) {
            Some(c) => c[1].to_string(),
            None => continue,
        };

        let size: i64 = SIZE_RE
            .captures(row)
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(-1);

        // rel is the URL-encoded name relative to the current directory.
        let rel = &href[2..]; // strip leading "./"
        let decoded = percent_decode(rel);

        if href.ends_with('/') || size < 0 {
            // Directory entry: queue for recursive crawl.
            // Keep the path URL-encoded so ureq can fetch it directly.
            subdirs.push(format!("{}{}", path, rel));
        } else if decoded.ends_with(".mdx") && size > 0 {
            let name = decoded.trim_end_matches(".mdx").to_string();
            // Build the full download URL (path stays URL-encoded).
            let file_url = format!("{}{}{}", base_url.trim_end_matches('/'), path, rel);
            entries.push(DictEntry {
                name,
                url: file_url,
                size: size as u64,
                category: category.clone(),
            });
        }
    }

    for sub in subdirs {
        eprintln!("  indexing {}{}", base_url, sub);
        std::thread::sleep(std::time::Duration::from_millis(CRAWL_DELAY_MS));
        crawl_dir(base_url, &sub, entries, depth + 1)?;
    }

    Ok(())
}

/// Derive a human-readable category string from a URL-encoded path.
/// "/按词典语种来分类/英语/普通词典/" → "按词典语种来分类 / 英语 / 普通词典"
fn path_to_category(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode(s))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Percent-decode a URL-encoded string into UTF-8.
fn percent_decode(s: &str) -> String {
    let src = s.as_bytes();
    let mut buf = Vec::with_capacity(src.len());
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
