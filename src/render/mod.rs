use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;
use termimad::{
    MadSkin, StyledChar,
    crossterm::style::{Attribute, Color},
};

pub trait Render {
    fn render(&self, content: impl AsRef<str>) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct DefaultRender;

// Matches " N." (space + one-or-more digits + dot).
// The caller checks that the character after the dot is not a digit, so
// decimal numbers like " 1.5" are excluded without needing lookahead.
static INLINE_NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" \d+\.").unwrap());

fn build_skin() -> MadSkin {
    let mut skin = MadSkin::default();

    // H1: bright gold + underline — primary headword
    skin.headers[0].compound_style.set_fg(Color::Rgb {
        r: 255,
        g: 215,
        b: 0,
    });
    skin.headers[0].compound_style.add_attr(Attribute::Bold);
    skin.headers[0]
        .compound_style
        .add_attr(Attribute::Underlined);

    // H2: coral orange — POS / section header
    skin.headers[1].compound_style.set_fg(Color::Rgb {
        r: 255,
        g: 110,
        b: 60,
    });
    skin.headers[1].compound_style.add_attr(Attribute::Bold);

    // H3: warm yellow — sub-section
    skin.headers[2].compound_style.set_fg(Color::Rgb {
        r: 230,
        g: 200,
        b: 70,
    });
    skin.headers[2].compound_style.add_attr(Attribute::Bold);

    // H4–H6: soft lavender
    for h in &mut skin.headers[3..] {
        h.compound_style.set_fg(Color::Rgb {
            r: 180,
            g: 155,
            b: 230,
        });
    }

    // Italic (phonetics, example sentences): sky cyan
    skin.italic.set_fg(Color::Rgb {
        r: 80,
        g: 210,
        b: 235,
    });

    // Bold (emphasis, POS labels): warm peach
    skin.bold.set_fg(Color::Rgb {
        r: 255,
        g: 200,
        b: 120,
    });
    skin.bold.add_attr(Attribute::Bold);

    // Inline code: mint green
    skin.inline_code.set_fg(Color::Rgb {
        r: 80,
        g: 220,
        b: 140,
    });

    // Bullet marker: sky blue ›
    skin.bullet = StyledChar::from_fg_char(
        Color::Rgb {
            r: 60,
            g: 180,
            b: 255,
        },
        '›',
    );

    // Block-quote bar: soft indigo │
    skin.quote_mark = StyledChar::from_fg_char(
        Color::Rgb {
            r: 120,
            g: 100,
            b: 220,
        },
        '│',
    );

    // Horizontal rule: dim grey ─
    skin.horizontal_rule = StyledChar::from_fg_char(
        Color::Rgb {
            r: 70,
            g: 70,
            b: 80,
        },
        '─',
    );

    skin
}

/// Split a plain-text paragraph that contains inline numbered definitions.
///
/// Some MDict dictionaries encode multiple definitions as inline text:
///   "1.foo 2.bar 3.baz"
/// This function splits at each " N." boundary and returns the individual items.
/// Returns `None` if the line does not contain an inline numbered list.
fn split_inline_numbered<'a>(para: &'a str) -> Option<Vec<&'a str>> {
    let first = para.trim();
    // Must start with a digit to be a candidate.
    if !first.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    // Collect split positions: matches of " N." where the char after the dot
    // is not another digit (avoids splitting on decimal numbers like " 1.5").
    let split_at: Vec<usize> = INLINE_NUM_RE
        .find_iter(first)
        .filter(|m| {
            !first
                .as_bytes()
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
    let mut last = 0usize;
    for pos in split_at {
        let item = first[last..pos].trim();
        if !item.is_empty() {
            items.push(item);
        }
        last = pos + 1; // skip the leading space; "N." stays with the next item
    }
    let tail = first[last..].trim();
    if !tail.is_empty() {
        items.push(tail);
    }

    if items.len() > 1 { Some(items) } else { None }
}

/// Normalize markdown from html2text for better terminal display.
///
/// - Inline numbered definitions ("1.foo 2.bar") are split into separate lines.
/// - A blank line is inserted before markdown list items that immediately follow
///   non-blank lines, so termimad renders each item as a distinct visual block.
fn normalize_lists(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 256);
    let mut prev_blank = true;

    for line in md.lines() {
        let trimmed = line.trim_start();

        // Try to split inline numbered definitions.
        if let Some(items) = split_inline_numbered(trimmed) {
            if !prev_blank {
                out.push('\n');
            }
            for item in items {
                out.push_str(item);
                out.push('\n');
            }
            out.push('\n');
            prev_blank = true;
            continue;
        }

        // Ensure a blank line precedes markdown list markers.
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

/// Returns true for markdown numbered list items like "1. " or "2) ".
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

impl Render for DefaultRender {
    fn render(&self, content: impl AsRef<str>) -> Result<String> {
        // Use a very large width so html2text never wraps mid-paragraph;
        // our normalize_lists pass and termimad handle final layout.
        let md = html2text::from_read(content.as_ref().as_bytes(), 10_000)?;
        let md = normalize_lists(&md);
        let skin = build_skin();
        Ok(skin.term_text(&md).to_string())
    }
}
