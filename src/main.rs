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

use std::{collections::HashSet, io::{Read, Write}, path::PathBuf, sync::{Arc, Mutex}};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color as TuiColor, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use yadict::parser;
use yadict::registry::{DictEntry, LocalRegistry, Registry};
use yadict::render::{DefaultRender, Render};

#[derive(Parser)]
#[command(name = "yadict", about = "MDict dictionary lookup tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Translate a word by querying all dictionaries in ~/.yadict/registry/<name>/
    #[command(alias = "t")]
    Translate {
        /// Word to look up
        word: String,

        /// Print raw Markdown instead of rendered terminal output
        #[arg(long)]
        markdown: bool,

        /// Print raw HTML instead of rendered terminal output
        #[arg(long)]
        html: bool,

        /// Registry name to query (scans ~/.yadict/registry/<name>/)
        #[arg(short = 'r', long = "registry", default_value = "default")]
        registry: String,
    },

    /// List all installed dictionaries in ~/.yadict/registry/<name>/
    List {
        /// Registry name to list (scans ~/.yadict/registry/<name>/)
        #[arg(short = 'r', long = "registry", default_value = "default")]
        registry: String,
    },

    /// Remove an installed dictionary from ~/.yadict/registry/<registry>/
    Remove {
        /// Dictionary name to remove (filename without .mdx extension)
        name: String,

        /// Registry name (scans ~/.yadict/registry/<name>/)
        #[arg(short = 'r', long = "registry", default_value = "default")]
        registry: String,
    },

    /// Browse and search the remote dictionary index (mdx.mdict.org)
    #[command(alias = "r")]
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },
}

#[derive(Subcommand)]
enum RegistryAction {
    /// List available dictionaries with a browsable TUI, optionally pre-filtered
    List {
        /// Case-insensitive substring to pre-filter by name or category
        query: Option<String>,
    },

    /// Search dictionaries by name, category, or registry name (plain output)
    Search { query: String },

    /// Reload all entries from installed registry YAML files
    Refresh,

    /// Install a registry from a local YAML file or HTTP(S) URL
    Add {
        /// Local file path or HTTP(S) URL to a registry YAML
        src: String,
    },
}

/// URL-keyed disk cache. Index is stored in `~/.yadict/cache.tsv` (tab-separated: url\tabsolute_path).
struct CacheStore {
    index_path: PathBuf,
}

impl CacheStore {
    fn new(home: &std::path::Path) -> Self {
        Self {
            index_path: home.join("cache.tsv"),
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

    /// Remove all cache entries whose stored path matches `path`.
    fn remove_by_path(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = match std::fs::read_to_string(&self.index_path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        let kept: Vec<&str> = content
            .lines()
            .filter(|line| {
                line.split_once('\t')
                    .map(|(_, p)| PathBuf::from(p) != path)
                    .unwrap_or(true)
            })
            .collect();
        let out = if kept.is_empty() {
            String::new()
        } else {
            format!("{}\n", kept.join("\n"))
        };
        std::fs::write(&self.index_path, out)?;
        Ok(())
    }
}

/// Download selected entries into `<home>/registry/<registry>/<name>.mdx` in parallel.
/// Already-cached files are skipped. Errors are reported via the progress bar.
fn download_files(entries: &[&DictEntry], home: &std::path::Path) -> anyhow::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let store = Arc::new(Mutex::new(CacheStore::new(home)));
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()?,
    );

    let mp = Arc::new(MultiProgress::new());
    let sty = ProgressStyle::with_template(
        " {spinner:.cyan} {msg:<40} [{bar:38.cyan/white}] {bytes:>10}/{total_bytes:<10}  {bytes_per_sec:>12}  eta {eta}",
    )
    .unwrap()
    .progress_chars("█▓░")
    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");

    std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .iter()
            .map(|entry| {
                let store = Arc::clone(&store);
                let client = Arc::clone(&client);
                let mp = Arc::clone(&mp);
                let sty = sty.clone();
                let url = entry.url.clone();
                let name = entry.name.clone();
                let dest = home
                    .join("registry")
                    .join(&entry.registry)
                    .join(format!("{}.mdx", entry.name));
                s.spawn(move || {
                    let pb = mp.add(ProgressBar::new(0));
                    pb.set_style(sty);
                    pb.set_message(name.clone());
                    pb.enable_steady_tick(std::time::Duration::from_millis(80));

                    if let Some(cached) = store.lock().unwrap().lookup(&url) {
                        debug!("using cached file: {}", cached.display());
                        pb.finish_with_message(format!("{name}  (already installed)"));
                        return;
                    }
                    if let Some(parent) = dest.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            pb.abandon_with_message(format!("{name}  ✗ {e}"));
                            return;
                        }
                    }
                    match fetch_file(&url, &dest, &client, &pb) {
                        Ok(()) => {
                            if let Err(e) = store.lock().unwrap().insert(&url, &dest) {
                                pb.println(format!("Warning: failed to record cache entry: {e}"));
                            }
                            pb.finish_with_message(format!("{name}  ✓"));
                        }
                        Err(e) => {
                            pb.abandon_with_message(format!("{name}  ✗ {e}"));
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    });
    Ok(())
}

/// Fetch a URL into a `.part` temp file beside `dest`, then rename on success.
/// The destination never contains a partial download; the temp file is cleaned
/// up automatically if the download fails.
fn fetch_file(
    url: &str,
    dest: &std::path::Path,
    client: &reqwest::blocking::Client,
    pb: &ProgressBar,
) -> anyhow::Result<()> {
    let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".part");
    let tmp = dest.with_file_name(tmp_name);

    let result = (|| -> anyhow::Result<()> {
        let mut resp = client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| anyhow!("Request failed: {e}"))?;
        if let Some(len) = resp.content_length() {
            pb.set_length(len);
        }
        let mut file = std::fs::File::create(&tmp)?;
        let mut buf = [0u8; 65536];
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            pb.inc(n as u64);
        }
        drop(file);
        std::fs::rename(&tmp, dest)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Find all `.mdx` files under `<home>/registry/<registry_name>/`.
fn find_mdx_files(home: &std::path::Path, registry_name: &str) -> anyhow::Result<Vec<PathBuf>> {
    let dir = home.join("registry").join(registry_name);
    let mut paths = Vec::new();
    if !dir.exists() {
        return Ok(paths);
    }
    for entry in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().is_some_and(|ext| ext == "mdx") {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
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

// ── Tree data types ───────────────────────────────────────────────────────────

struct CategoryNode {
    segment: String,
    full_path: String,
    children: Vec<CategoryNode>,
    entry_indices: Vec<usize>,
}

#[derive(Clone)]
enum VisibleRow {
    Category {
        segment: String,
        full_path: String,
        depth: usize,
        expanded: bool,
        entry_count: usize,
    },
    Entry {
        entry_idx: usize,
        depth: usize,
        show_category: bool,
    },
}

// ── Tree helpers ──────────────────────────────────────────────────────────────

fn build_category_tree(entries: &[&DictEntry]) -> Vec<CategoryNode> {
    let mut roots: Vec<CategoryNode> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if entry.categories.is_empty() {
            insert_into_tree(&mut roots, &["(uncategorized)"], "", i);
        } else {
            for cat in &entry.categories {
                let segs: Vec<&str> = cat.split(" / ").filter(|s| !s.is_empty()).collect();
                let segs: &[&str] = if segs.is_empty() { &["(uncategorized)"] } else { &segs };
                insert_into_tree(&mut roots, segs, "", i);
            }
        }
    }
    roots
}

fn insert_into_tree(nodes: &mut Vec<CategoryNode>, segs: &[&str], parent_path: &str, idx: usize) {
    let seg = segs[0];
    let full_path = if parent_path.is_empty() {
        seg.to_string()
    } else {
        format!("{} / {}", parent_path, seg)
    };
    let pos = nodes
        .iter()
        .position(|n| n.segment == seg)
        .unwrap_or_else(|| {
            nodes.push(CategoryNode {
                segment: seg.to_string(),
                full_path: full_path.clone(),
                children: Vec::new(),
                entry_indices: Vec::new(),
            });
            nodes.len() - 1
        });
    if segs.len() == 1 {
        nodes[pos].entry_indices.push(idx);
    } else {
        let fp = nodes[pos].full_path.clone();
        insert_into_tree(&mut nodes[pos].children, &segs[1..], &fp, idx);
    }
}

fn count_tree_entries(node: &CategoryNode) -> usize {
    node.entry_indices.len() + node.children.iter().map(count_tree_entries).sum::<usize>()
}

fn flatten_tree(
    nodes: &[CategoryNode],
    depth: usize,
    expanded: &HashSet<String>,
    out: &mut Vec<VisibleRow>,
) {
    for node in nodes {
        let is_expanded = expanded.contains(&node.full_path);
        out.push(VisibleRow::Category {
            segment: node.segment.clone(),
            full_path: node.full_path.clone(),
            depth,
            expanded: is_expanded,
            entry_count: count_tree_entries(node),
        });
        if is_expanded {
            flatten_tree(&node.children, depth + 1, expanded, out);
            for &entry_idx in &node.entry_indices {
                out.push(VisibleRow::Entry {
                    entry_idx,
                    depth: depth + 1,
                    show_category: false,
                });
            }
        }
    }
}

fn filter_entries(entries: &[&DictEntry], query: &str) -> Vec<VisibleRow> {
    let q = query.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.name.to_lowercase().contains(&q)
                || e.categories.iter().any(|c| c.to_lowercase().contains(&q))
        })
        .map(|(idx, _)| VisibleRow::Entry {
            entry_idx: idx,
            depth: 0,
            show_category: true,
        })
        .collect()
}

fn make_list_item(
    row: &VisibleRow,
    entries: &[&DictEntry],
    selected_urls: &HashSet<String>,
    installed: &HashSet<String>,
) -> ListItem<'static> {
    match row {
        VisibleRow::Category {
            segment,
            depth,
            expanded,
            entry_count,
            ..
        } => {
            let arrow = if *expanded { "▼ " } else { "▶ " };
            ListItem::new(Line::from(vec![
                Span::raw("  ".repeat(*depth)),
                Span::styled(arrow.to_string(), Style::default().fg(TuiColor::Cyan)),
                Span::styled(
                    segment.clone(),
                    Style::default()
                        .fg(TuiColor::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ({} dicts)", entry_count),
                    Style::default().fg(TuiColor::Rgb(100, 100, 120)),
                ),
            ]))
        }
        VisibleRow::Entry {
            entry_idx,
            depth,
            show_category,
        } => {
            let e = entries[*entry_idx];
            let check = if selected_urls.contains(&e.url) {
                "[x]"
            } else {
                "[ ]"
            };
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw("  ".repeat(*depth)),
                Span::styled(check.to_string(), Style::default().fg(TuiColor::Yellow)),
                Span::raw("  "),
            ];
            if *show_category && !e.categories.is_empty() {
                spans.push(Span::styled(
                    format!("[{}]  ", e.categories.join(", ")),
                    Style::default().fg(TuiColor::Rgb(120, 120, 140)),
                ));
            }
            spans.push(Span::raw(e.name.clone()));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                e.size_human(),
                Style::default().fg(TuiColor::Rgb(80, 180, 80)),
            ));
            if installed.contains(&e.url) {
                spans.push(Span::styled(
                    "  ✓".to_string(),
                    Style::default().fg(TuiColor::Rgb(80, 220, 140)).add_modifier(Modifier::BOLD),
                ));
            }
            ListItem::new(Line::from(spans))
        }
    }
}

// ── TUI entry point ───────────────────────────────────────────────────────────

fn centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    ratatui::layout::Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

/// Show a confirmation popup listing dictionaries to download and/or remove.
/// Returns true if the user confirms, false if they cancel.
fn run_confirm_tui(to_download: &[&DictEntry], to_remove: &[&DictEntry]) -> anyhow::Result<bool> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = confirm_loop(&mut terminal, to_download, to_remove);

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn confirm_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    to_download: &[&DictEntry],
    to_remove: &[&DictEntry],
) -> anyhow::Result<bool> {
    loop {
        terminal.draw(|f| {
            let area = f.area();

            // Calculate content height: section headers + items + blank separator
            let mut content_h: u16 = 0;
            if !to_download.is_empty() {
                content_h += 1 + to_download.len() as u16;
            }
            if !to_remove.is_empty() {
                if !to_download.is_empty() { content_h += 1; }
                content_h += 1 + to_remove.len() as u16;
            }
            let popup_h = (content_h + 4).min(area.height.saturating_sub(2));
            let popup_w = 72u16.min(area.width.saturating_sub(4));
            let popup = centered_rect(popup_w, popup_h, area);

            f.render_widget(Clear, popup);

            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Confirm changes ")
                .title_style(Style::default().fg(TuiColor::Yellow).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(TuiColor::Black));
            let inner = block.inner(popup);
            f.render_widget(block, popup);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(inner);

            let mut items: Vec<ListItem> = Vec::new();

            if !to_download.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  Download ({}):", to_download.len()),
                    Style::default().fg(TuiColor::Cyan).add_modifier(Modifier::BOLD),
                ))));
                for e in to_download {
                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("    • "),
                        Span::raw(e.name.clone()),
                        Span::raw("  "),
                        Span::styled(e.size_human(), Style::default().fg(TuiColor::Rgb(80, 180, 80))),
                    ])));
                }
            }

            if !to_remove.is_empty() {
                if !to_download.is_empty() {
                    items.push(ListItem::new(Line::default()));
                }
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  Remove ({}):", to_remove.len()),
                    Style::default().fg(TuiColor::Red).add_modifier(Modifier::BOLD),
                ))));
                for e in to_remove {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("    • ", Style::default().fg(TuiColor::Red)),
                        Span::styled(e.name.clone(), Style::default().fg(TuiColor::Rgb(255, 100, 100))),
                    ])));
                }
            }

            f.render_widget(List::new(items), chunks[0]);
            f.render_widget(
                Paragraph::new(Span::styled(
                    "  Enter / y: confirm    Esc / n: cancel",
                    Style::default().fg(TuiColor::DarkGray),
                )),
                chunks[1],
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => return Ok(true),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => return Ok(false),
                _ => {}
            }
        }
    }
}

/// Launch the multi-select registry TUI.
/// Returns Some(urls) when the user confirms, None when they cancel (q/Esc).
/// Restores the terminal before returning, even on error.
fn run_registry_tui(entries: &[&DictEntry], installed: &HashSet<String>) -> anyhow::Result<Option<Vec<String>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = tui_select_loop(&mut terminal, entries, installed);

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn tui_select_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    entries: &[&DictEntry],
    installed: &HashSet<String>,
) -> anyhow::Result<Option<Vec<String>>> {
    let tree = build_category_tree(entries);
    // Expand all top-level nodes by default.
    let mut expanded: HashSet<String> = tree.iter().map(|n| n.full_path.clone()).collect();
    let mut selected_urls: HashSet<String> = installed.iter().cloned().collect();
    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut search_query = String::new();
    let mut search_active = false;

    loop {
        let visible: Vec<VisibleRow> = if search_query.is_empty() {
            let mut rows = Vec::new();
            flatten_tree(&tree, 0, &expanded, &mut rows);
            rows
        } else {
            filter_entries(entries, &search_query)
        };

        // Clamp cursor when the list shrinks (e.g. after search filter narrows results).
        if let Some(i) = list_state.selected() {
            if !visible.is_empty() && i >= visible.len() {
                list_state.select(Some(visible.len() - 1));
            }
        }
        if list_state.selected().is_none() && !visible.is_empty() {
            list_state.select(Some(0));
        }

        let sel_count = selected_urls.len();

        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
                .split(area);

            // Search bar
            let search_bar = if search_active {
                Line::from(vec![
                    Span::styled(" / ", Style::default().fg(TuiColor::Yellow)),
                    Span::raw(search_query.clone()),
                    Span::styled("▌", Style::default().fg(TuiColor::Yellow)),
                ])
            } else {
                Line::from(Span::styled(
                    " Press / to search",
                    Style::default().fg(TuiColor::DarkGray),
                ))
            };
            f.render_widget(Paragraph::new(search_bar), chunks[0]);

            // Directory tree / search results
            let title = format!(
                " yadict registry — {} selected / {} total ",
                sel_count,
                entries.len()
            );
            let items: Vec<ListItem> = visible
                .iter()
                .map(|row| make_list_item(row, entries, &selected_urls, installed))
                .collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(title))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("  ");
            f.render_stateful_widget(list, chunks[1], &mut list_state);

            // Key-binding hint
            let hint = if search_active {
                "  ESC: clear search  ↑↓/jk: move  SPACE: toggle  ENTER: download selected  q: quit"
            } else {
                "  /: search  ↑↓/jk: move  l/h→←: expand  SPACE: toggle  a: all  ENTER: download  q: quit"
            };
            f.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(TuiColor::DarkGray))),
                chunks[2],
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if search_active {
                match key.code {
                    KeyCode::Esc => {
                        search_query.clear();
                        search_active = false;
                        list_state.select(Some(0));
                    }
                    KeyCode::Backspace => {
                        search_query.pop();
                        list_state.select(Some(0));
                    }
                    KeyCode::Enter => return Ok(Some(selected_urls.into_iter().collect())),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(i) = list_state.selected() {
                            if i > 0 {
                                list_state.select(Some(i - 1));
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(i) = list_state.selected() {
                            if i + 1 < visible.len() {
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
                            list_state.select(Some((i + 20).min(visible.len().saturating_sub(1))));
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some(i) = list_state.selected() {
                            if let Some(VisibleRow::Entry { entry_idx, .. }) = visible.get(i) {
                                let url = entries[*entry_idx].url.clone();
                                if !selected_urls.remove(&url) {
                                    selected_urls.insert(url);
                                }
                                if i + 1 < visible.len() {
                                    list_state.select(Some(i + 1));
                                }
                            }
                        }
                    }
                    // Any other character appends to the search query.
                    KeyCode::Char(c) => {
                        search_query.push(c);
                        list_state.select(Some(0));
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Char('/') => {
                        search_active = true;
                        search_query.clear();
                        list_state.select(Some(0));
                    }
                    KeyCode::Enter => return Ok(Some(selected_urls.into_iter().collect())),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(i) = list_state.selected() {
                            if i > 0 {
                                list_state.select(Some(i - 1));
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(i) = list_state.selected() {
                            if i + 1 < visible.len() {
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
                            list_state.select(Some((i + 20).min(visible.len().saturating_sub(1))));
                        }
                    }
                    // Expand/collapse
                    KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(i) = list_state.selected() {
                            if let Some(VisibleRow::Category { full_path, .. }) = visible.get(i) {
                                expanded.insert(full_path.clone());
                            }
                        }
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        if let Some(i) = list_state.selected() {
                            if let Some(VisibleRow::Category { full_path, .. }) = visible.get(i) {
                                expanded.remove(full_path.as_str());
                            }
                        }
                    }
                    // Space: toggle expand on categories, toggle selection on entries.
                    KeyCode::Char(' ') => {
                        if let Some(i) = list_state.selected() {
                            match visible.get(i) {
                                Some(VisibleRow::Category {
                                    full_path,
                                    expanded: is_exp,
                                    ..
                                }) => {
                                    if *is_exp {
                                        expanded.remove(full_path.as_str());
                                    } else {
                                        expanded.insert(full_path.clone());
                                    }
                                }
                                Some(VisibleRow::Entry { entry_idx, .. }) => {
                                    let url = entries[*entry_idx].url.clone();
                                    if !selected_urls.remove(&url) {
                                        selected_urls.insert(url);
                                    }
                                    if i + 1 < visible.len() {
                                        list_state.select(Some(i + 1));
                                    }
                                }
                                None => {}
                            }
                        }
                    }
                    // Select / deselect all currently visible entries.
                    KeyCode::Char('a') => {
                        let visible_urls: Vec<String> = visible
                            .iter()
                            .filter_map(|row| {
                                if let VisibleRow::Entry { entry_idx, .. } = row {
                                    Some(entries[*entry_idx].url.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let all = visible_urls.iter().all(|u| selected_urls.contains(u));
                        if all {
                            for u in &visible_urls {
                                selected_urls.remove(u);
                            }
                        } else {
                            for u in visible_urls {
                                selected_urls.insert(u);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::try_init().ok();

    let default_render = DefaultRender;

    let cli = Cli::parse();

    match cli.command {
        Commands::Registry { action } => {
            let home = yadict_home()?;
            match action {
                RegistryAction::Add { src } => {
                    LocalRegistry::install(&src, &home)?;
                }
                RegistryAction::Refresh => {
                    let mut reg = LocalRegistry::new(&home);
                    reg.refresh()?;
                }
                RegistryAction::List { query } => {
                    let reg = LocalRegistry::new(&home);
                    if reg.is_stale() {
                        eprintln!(
                            "No registries installed. Use `yadict registry add <path>` to install one."
                        );
                        return Ok(());
                    }
                    let entries: Vec<_> = match &query {
                        Some(q) => reg.search(q),
                        None => reg.entries().iter().collect(),
                    };
                    if entries.is_empty() {
                        println!("No dictionaries found.");
                    } else {
                        let installed: HashSet<String> = entries
                            .iter()
                            .filter(|e| {
                                home.join("registry")
                                    .join(&e.registry)
                                    .join(format!("{}.mdx", e.name))
                                    .exists()
                            })
                            .map(|e| e.url.clone())
                            .collect();
                        let Some(urls) = run_registry_tui(&entries, &installed)? else {
                            return Ok(()); // user cancelled
                        };
                        let url_set: HashSet<&str> =
                            urls.iter().map(|s| s.as_str()).collect();
                        let to_download: Vec<&DictEntry> = entries
                            .iter()
                            .filter(|e| url_set.contains(e.url.as_str()) && !installed.contains(&e.url))
                            .copied()
                            .collect();
                        let to_remove: Vec<&DictEntry> = entries
                            .iter()
                            .filter(|e| !url_set.contains(e.url.as_str()) && installed.contains(&e.url))
                            .copied()
                            .collect();
                        if to_download.is_empty() && to_remove.is_empty() {
                            return Ok(()); // no changes, exit silently
                        }
                        if run_confirm_tui(&to_download, &to_remove)? {
                            for e in &to_remove {
                                let p = home
                                    .join("registry")
                                    .join(&e.registry)
                                    .join(format!("{}.mdx", e.name));
                                if p.exists() {
                                    std::fs::remove_file(&p)?;
                                    CacheStore::new(&home).remove_by_path(&p)?;
                                }
                            }
                            download_files(&to_download, &home)?;
                        }
                    }
                }
                RegistryAction::Search { query } => {
                    let reg = LocalRegistry::new(&home);
                    if reg.is_stale() {
                        eprintln!(
                            "No registries installed. Use `yadict registry add <path>` to install one."
                        );
                        return Ok(());
                    }
                    let entries = reg.search(&query);
                    if entries.is_empty() {
                        println!("No dictionaries found.");
                    } else {
                        for e in &entries {
                            print!("{}", SetForegroundColor(Color::Rgb { r: 100, g: 100, b: 120 }));
                            print!("[{}]", e.categories.join(", "));
                            print!("{}", ResetColor);
                            print!(" {}", e.name);
                            print!("{}", SetForegroundColor(Color::Rgb { r: 80, g: 180, b: 80 }));
                            println!("  ({})", e.registry);
                            print!("{}", ResetColor);
                        }
                        println!("\n{} dictionaries found.", entries.len());
                    }
                }
            }
        }
        Commands::Remove { name, registry } => {
            let home = yadict_home()?;
            let path = home
                .join("registry")
                .join(&registry)
                .join(format!("{}.mdx", name));
            if !path.exists() {
                eprintln!("Dictionary '{name}' not found in registry '{registry}'.");
                return Ok(());
            }
            std::fs::remove_file(&path)?;
            CacheStore::new(&home).remove_by_path(&path)?;
            println!("Removed '{name}' from registry '{registry}'.");
        }
        Commands::List { registry } => {
            let home = yadict_home()?;
            let paths = find_mdx_files(&home, &registry)?;
            if paths.is_empty() {
                println!(
                    "No dictionaries installed in registry '{registry}'. Use `yadict registry list` to browse and install."
                );
            } else {
                for path in &paths {
                    println!("{}", path.file_stem().unwrap_or_default().to_string_lossy());
                }
            }
        }
        Commands::Translate { word, markdown, html, registry } => {
            let home = yadict_home()?;
            let paths = find_mdx_files(&home, &registry)?;

            if paths.is_empty() {
                eprintln!(
                    "No dictionaries found in registry '{registry}'. Use `yadict registry list` to browse and install."
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
                    let value = record.value();
                    let output = if html {
                        value.to_string()
                    } else if markdown {
                        html2text::from_read(value.as_bytes(), 80)
                            .unwrap_or_else(|_| value.to_string())
                    } else {
                        default_render.render(value)?
                    };
                    results.push(output);
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
