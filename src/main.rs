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
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color as TuiColor, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use yadict::parser;
use yadict::registry::{DictEntry, MdictOrgRegistry, Registry};
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

        /// Print raw Markdown instead of rendered terminal output
        #[arg(long)]
        markdown: bool,
    },

    /// Download a remote .mdx dictionary to ~/.yadict/mdicts/
    Add {
        /// HTTP(S) URL of the .mdx file to download
        url: String,
    },

    /// List all installed dictionaries in ~/.yadict/mdicts/
    List,

    /// Browse and search the remote dictionary index (mdx.mdict.org)
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },
}

#[derive(Subcommand)]
enum RegistryAction {
    /// List available dictionaries, optionally filtered by a search term
    List {
        /// Case-insensitive substring to filter by name or category
        query: Option<String>,
    },

    /// Search dictionaries by name or category
    Search {
        query: String,
    },

    /// Re-crawl mdx.mdict.org and rebuild the local index cache
    Refresh,
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

/// Launch the multi-select registry TUI and return URLs chosen by the user.
/// Restores the terminal before returning, even on error.
fn run_registry_tui(entries: &[&DictEntry]) -> anyhow::Result<Vec<String>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = tui_select_loop(&mut terminal, entries);

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn tui_select_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    entries: &[&DictEntry],
) -> anyhow::Result<Vec<String>> {
    let mut selected = vec![false; entries.len()];
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        let sel_count = selected.iter().filter(|&&s| s).count();

        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);

            let title = format!(
                " yadict registry — {} selected / {} total ",
                sel_count,
                entries.len()
            );
            let items: Vec<ListItem> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let check = if selected[i] { "[x]" } else { "[ ]" };
                    ListItem::new(Line::from(vec![
                        Span::styled(check, Style::default().fg(TuiColor::Yellow)),
                        Span::raw("  "),
                        Span::styled(
                            format!("[{}]", e.category),
                            Style::default().fg(TuiColor::Rgb(120, 120, 140)),
                        ),
                        Span::raw("  "),
                        Span::raw(e.name.clone()),
                        Span::raw("  "),
                        Span::styled(
                            e.size_human(),
                            Style::default().fg(TuiColor::Rgb(80, 180, 80)),
                        ),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(title))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("  ");

            f.render_stateful_widget(list, chunks[0], &mut list_state);

            let hint = "  ↑↓/jk: move  PgUp/PgDn: scroll  SPACE: toggle  a: all/none  ENTER: add selected  q: quit";
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default().fg(TuiColor::DarkGray),
                ))),
                chunks[1],
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(vec![]),
                KeyCode::Enter => {
                    return Ok(entries
                        .iter()
                        .enumerate()
                        .filter_map(|(i, e)| if selected[i] { Some(e.url.clone()) } else { None })
                        .collect());
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(i) = list_state.selected() {
                        if i > 0 {
                            list_state.select(Some(i - 1));
                        }
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(i) = list_state.selected() {
                        if i + 1 < entries.len() {
                            list_state.select(Some(i + 1));
                        }
                    }
                }
                KeyCode::PageUp => {
                    if let Some(i) = list_state.selected() {
                        list_state.select(Some(i.saturating_sub(20)));
                    }
                }
                KeyCode::PageDown => {
                    if let Some(i) = list_state.selected() {
                        list_state.select(Some((i + 20).min(entries.len().saturating_sub(1))));
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(i) = list_state.selected() {
                        selected[i] = !selected[i];
                        if i + 1 < entries.len() {
                            list_state.select(Some(i + 1));
                        }
                    }
                }
                KeyCode::Char('a') => {
                    let all = selected.iter().all(|&s| s);
                    selected.iter_mut().for_each(|s| *s = !all);
                }
                _ => {}
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::try_init().ok();

    let default_render = DefaultRender::default();

    let cli = Cli::parse();

    match cli.command {
        Commands::Add { url } => {
            download_to_cache(&url)?;
        }
        Commands::Registry { action } => {
            let mut reg = MdictOrgRegistry::new(&yadict_home()?);
            match action {
                RegistryAction::Refresh => {
                    reg.refresh()?;
                }
                RegistryAction::List { query } => {
                    if reg.is_stale() {
                        eprintln!("Registry is empty. Run `yadict registry refresh` to build the index.");
                        return Ok(());
                    }
                    let entries: Vec<_> = match &query {
                        Some(q) => reg.search(q),
                        None => reg.entries().iter().collect(),
                    };
                    if entries.is_empty() {
                        println!("No dictionaries found.");
                    } else {
                        let urls = run_registry_tui(&entries)?;
                        for url in &urls {
                            download_to_cache(url)?;
                        }
                    }
                }
                RegistryAction::Search { query } => {
                    if reg.is_stale() {
                        eprintln!("Registry is empty. Run `yadict registry refresh` to build the index.");
                        return Ok(());
                    }
                    let entries = reg.search(&query);
                    if entries.is_empty() {
                        println!("No dictionaries found.");
                    } else {
                        for e in &entries {
                            print!("{}", SetForegroundColor(Color::Rgb { r: 100, g: 100, b: 120 }));
                            print!("[{}]", e.category);
                            print!("{}", ResetColor);
                            print!(" {}", e.name);
                            print!("{}", SetForegroundColor(Color::Rgb { r: 80, g: 180, b: 80 }));
                            println!("  {}", e.size_human());
                            print!("{}", ResetColor);
                        }
                        println!("\n{} dictionaries found.", entries.len());
                    }
                }
            }
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
        Commands::Translate { word, markdown } => {
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
                        let output = if markdown {
                            html2text::from_read(value.as_bytes(), 80)
                                .unwrap_or_else(|_| value.into_owned())
                        } else {
                            default_render.render(value)?
                        };
                        results.push(output);
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
