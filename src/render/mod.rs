use anyhow::Result;
use termimad::{
    crossterm::style::{Attribute, Color},
    MadSkin, StyledChar,
};

pub trait Render {
    fn render(&self, content: impl AsRef<str>) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct DefaultRender;

fn build_skin() -> MadSkin {
    let mut skin = MadSkin::default();

    // Headings (h1-h6): bold amber, most prominent
    for h in &mut skin.headers {
        h.compound_style.set_fg(Color::Rgb {
            r: 255,
            g: 200,
            b: 0,
        });
        h.compound_style.add_attr(Attribute::Bold);
    }

    // Italic (phonetics): soft cyan
    skin.italic.set_fg(Color::Rgb {
        r: 100,
        g: 220,
        b: 220,
    });

    // Bold: bright white
    skin.bold.set_fg(Color::White);
    skin.bold.add_attr(Attribute::Bold);

    // Inline code: light green
    skin.inline_code.set_fg(Color::Rgb {
        r: 100,
        g: 220,
        b: 130,
    });

    // Bullet marker: muted grey, unobtrusive
    skin.bullet = StyledChar::from_fg_char(
        Color::Rgb {
            r: 120,
            g: 120,
            b: 140,
        },
        '›',
    );

    // Block quote marker: blue-grey vertical bar
    skin.quote_mark = StyledChar::from_fg_char(
        Color::Rgb {
            r: 80,
            g: 130,
            b: 180,
        },
        '│',
    );

    // Horizontal rule: dark grey
    skin.horizontal_rule = StyledChar::from_fg_char(
        Color::Rgb {
            r: 80,
            g: 80,
            b: 90,
        },
        '─',
    );

    skin
}

impl Render for DefaultRender {
    fn render(&self, content: impl AsRef<str>) -> Result<String> {
        // Use a large width so html2text does not wrap prematurely; termimad handles final layout.
        let md = html2text::from_read(content.as_ref().as_bytes(), 200)?;
        let skin = build_skin();
        Ok(skin.term_text(&md).to_string())
    }
}
