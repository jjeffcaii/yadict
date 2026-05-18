use anyhow::Result;
use comrak::{
    Arena, Options,
    nodes::{AstNode, ListType, NodeValue},
};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use regex::Regex;
use std::sync::LazyLock;

pub trait Render {
    fn render(&self, content: impl AsRef<str>) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct DefaultRender;

// Matches " N." (space + digits + dot). The caller filters out decimals like " 1.5"
// by checking that the byte after the dot is not a digit.
static INLINE_NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" \d+\.").unwrap());

// ── ANSI helpers ──────────────────────────────────────────────────────────────

fn fg(r: u8, g: u8, b: u8) -> String {
    format!("{}", SetForegroundColor(Color::Rgb { r, g, b }))
}

fn atr(a: Attribute) -> String {
    format!("{}", SetAttribute(a))
}

fn rst() -> String {
    format!("{}{}", ResetColor, SetAttribute(Attribute::Reset))
}

// ── Markdown preprocessing ────────────────────────────────────────────────────

/// Split a paragraph that contains inline numbered definitions ("1.foo 2.bar")
/// into individual items, excluding decimal numbers like "1.5".
fn split_inline_numbered(para: &str) -> Option<Vec<&str>> {
    let s = para.trim();
    if !s.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let split_at: Vec<usize> = INLINE_NUM_RE
        .find_iter(s)
        .filter(|m| {
            !s.as_bytes()
                .get(m.end())
                .copied()
                .map_or(false, |b| b.is_ascii_digit())
        })
        .map(|m| m.start())
        .collect();
    if split_at.is_empty() {
        return None;
    }
    let mut items: Vec<&str> = Vec::with_capacity(split_at.len() + 1);
    let mut last = 0;
    for pos in split_at {
        let item = s[last..pos].trim();
        if !item.is_empty() {
            items.push(item);
        }
        last = pos + 1;
    }
    let tail = s[last..].trim();
    if !tail.is_empty() {
        items.push(tail);
    }
    if items.len() > 1 { Some(items) } else { None }
}

/// Convert "N.text" → "N. text" so comrak parses it as a numbered list item.
fn to_md_list_item(item: &str) -> String {
    if let Some(dot) = item.find('.') {
        let pre = &item[..dot];
        if pre.bytes().all(|b| b.is_ascii_digit()) {
            return format!("{}. {}", pre, item[dot + 1..].trim_start());
        }
    }
    item.to_string()
}

/// Normalize html2text markdown output for cleaner rendering:
/// - Splits inline numbered definitions ("1.foo 2.bar") into proper ordered list items.
/// - Ensures blank lines before markdown list markers so comrak parses them correctly.
fn normalize_lists(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 256);
    let mut prev_blank = true;

    for line in md.lines() {
        let trimmed = line.trim_start();

        if let Some(items) = split_inline_numbered(trimmed) {
            if !prev_blank {
                out.push('\n');
            }
            for item in &items {
                out.push_str(&to_md_list_item(item));
                out.push('\n');
            }
            out.push('\n');
            prev_blank = true;
            continue;
        }

        let is_list_marker = trimmed.starts_with("* ")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("+ ")
            || is_numbered_md_item(trimmed);
        if is_list_marker && !prev_blank {
            out.push('\n');
        }

        out.push_str(line);
        out.push('\n');
        prev_blank = trimmed.is_empty();
    }
    out
}

fn is_numbered_md_item(s: &str) -> bool {
    let n = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    n > 0
        && s.as_bytes()
            .get(n)
            .copied()
            .map_or(false, |b| b == b'.' || b == b')')
        && s.as_bytes()
            .get(n + 1)
            .copied()
            .map_or(false, |b| b == b' ')
}

// ── AST renderer ─────────────────────────────────────────────────────────────

fn render_children<'a>(node: &'a AstNode<'a>, out: &mut String) {
    for child in node.children() {
        render_node(child, out);
    }
}

fn render_node<'a>(node: &'a AstNode<'a>, out: &mut String) {
    // Extract what we need from the RefCell inside the borrow scope, then release
    // the borrow so recursive render_children calls can re-borrow child nodes.
    enum Info {
        Text(String),
        SoftBreak,
        LineBreak,
        Paragraph,
        Heading(u8),
        Strong,
        Emph,
        Code(String),
        CodeBlock(String),
        ThematicBreak,
        BlockQuote,
        List { ordered: bool, start: usize },
        Link(String),
        Skip,
        Children,
    }

    let info = {
        let data = node.data.borrow();
        match &data.value {
            NodeValue::Text(s) => Info::Text(s.to_string()),
            NodeValue::SoftBreak => Info::SoftBreak,
            NodeValue::LineBreak => Info::LineBreak,
            NodeValue::Paragraph => Info::Paragraph,
            NodeValue::Heading(h) => Info::Heading(h.level),
            NodeValue::Strong => Info::Strong,
            NodeValue::Emph => Info::Emph,
            NodeValue::Code(c) => Info::Code(c.literal.clone()),
            NodeValue::CodeBlock(c) => Info::CodeBlock(c.literal.clone()),
            NodeValue::ThematicBreak => Info::ThematicBreak,
            NodeValue::BlockQuote => Info::BlockQuote,
            NodeValue::List(l) => Info::List {
                ordered: matches!(l.list_type, ListType::Ordered),
                start: l.start,
            },
            NodeValue::Link(l) => Info::Link(l.url.clone()),
            NodeValue::HtmlInline(_) | NodeValue::HtmlBlock(_) => Info::Skip,
            _ => Info::Children,
        }
    }; // borrow dropped here

    match info {
        Info::Text(s) => out.push_str(&s),
        Info::SoftBreak => out.push(' '),
        Info::LineBreak => out.push('\n'),
        Info::Skip => {}
        Info::Children => render_children(node, out),

        Info::Paragraph => {
            render_children(node, out);
            out.push('\n');
        }

        Info::Heading(level) => {
            out.push_str(&match level {
                1 => fg(255, 215, 0),
                2 => fg(255, 110, 60),
                3 => fg(230, 200, 70),
                _ => fg(180, 155, 230),
            });
            out.push_str(&atr(Attribute::Bold));
            if level == 1 {
                out.push_str(&atr(Attribute::Underlined));
            }
            render_children(node, out);
            out.push_str(&rst());
            out.push('\n');
        }

        Info::Strong => {
            out.push_str(&fg(255, 200, 120));
            out.push_str(&atr(Attribute::Bold));
            render_children(node, out);
            out.push_str(&rst());
        }

        Info::Emph => {
            out.push_str(&fg(80, 210, 235));
            out.push_str(&atr(Attribute::Italic));
            render_children(node, out);
            out.push_str(&rst());
        }

        Info::Code(literal) => {
            out.push_str(&fg(80, 220, 140));
            out.push_str(&literal);
            out.push_str(&rst());
        }

        Info::CodeBlock(literal) => {
            out.push_str(&fg(80, 220, 140));
            for line in literal.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&rst());
        }

        Info::ThematicBreak => {
            out.push_str(&fg(70, 70, 80));
            out.push_str(&"─".repeat(60));
            out.push_str(&rst());
            out.push('\n');
        }

        Info::BlockQuote => {
            for child in node.children() {
                out.push_str(&fg(120, 100, 220));
                out.push('│');
                out.push_str(&rst());
                out.push(' ');
                render_node(child, out);
            }
        }

        Info::List { ordered, start } => {
            for (i, item) in node.children().enumerate() {
                render_list_item(item, out, ordered, start + i);
            }
        }

        Info::Link(url) => {
            render_children(node, out);
            if !url.is_empty() {
                out.push_str(&fg(100, 100, 130));
                out.push_str(&format!(" ({})", url));
                out.push_str(&rst());
            }
        }
    }
}

/// Render a single list item, prefixed with a sky-blue bullet or a numbered label.
/// For tight items (single paragraph child) the paragraph's trailing newline is
/// suppressed so the item fits on one line.
fn render_list_item<'a>(node: &'a AstNode<'a>, out: &mut String, ordered: bool, num: usize) {
    if ordered {
        out.push_str(&fg(150, 150, 180));
        out.push_str(&format!("{}.", num));
        out.push_str(&rst());
    } else {
        out.push_str(&fg(60, 180, 255));
        out.push('›');
        out.push_str(&rst());
    }
    out.push(' ');

    let children: Vec<_> = node.children().collect();
    let tight =
        children.len() == 1 && matches!(children[0].data.borrow().value, NodeValue::Paragraph);

    if tight {
        // Render paragraph content inline without the paragraph's own trailing newline.
        render_children(children[0], out);
        out.push('\n');
    } else {
        for child in &children {
            render_node(child, out);
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

impl Render for DefaultRender {
    fn render(&self, content: impl AsRef<str>) -> Result<String> {
        // Use a very large width so html2text never wraps mid-paragraph.
        let md = html2text::from_read(content.as_ref().as_bytes(), 10_000)?;
        let md = normalize_lists(&md);

        let arena = Arena::new();
        let root = comrak::parse_document(&arena, &md, &Options::default());

        let mut out = String::new();
        render_node(root, &mut out);
        Ok(out)
    }
}
