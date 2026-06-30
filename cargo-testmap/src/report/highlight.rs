use anyhow::Result;
use std::path::Path;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, Theme, ThemeSet};
use syntect::html::{styled_line_to_highlighted_html, IncludeBackground};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Bundle of loaded syntax/theme state, created once and reused.
pub struct Highlighter {
    ss: SyntaxSet,
    themes: ThemeSet,
}

static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();

impl Highlighter {
    fn new() -> Self {
        Self {
            ss: SyntaxSet::load_defaults_newlines(),
            themes: ThemeSet::load_defaults(),
        }
    }

    pub fn get() -> &'static Highlighter {
        HIGHLIGHTER.get_or_init(Highlighter::new)
    }

    /// Resolve a theme name to an actual theme.
    ///
    /// Returns a helpful error listing every available theme (one per line)
    /// when `name` is unknown, instead of silently falling back to the
    /// default — so a typo can't quietly leave you with the wrong colors.
    pub fn resolve_theme(&self, name: Option<&str>) -> anyhow::Result<&Theme> {
        let default = "base16-ocean.dark";
        let name = name.unwrap_or(default);
        if let Some(t) = self.themes.themes.get(name) {
            return Ok(t);
        }
        let names = self.available_themes();
        anyhow::bail!(
            "unknown syntax theme `{name}`\n\navailable themes:\n{}",
            names.join("\n")
        );
    }

    /// All available theme names, sorted (one per line, copy-paste friendly).
    pub fn available_themes(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.themes.themes.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    fn syntax_for_path(&self, path: &str) -> Option<&SyntaxReference> {
        let p = Path::new(path);
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.is_empty()
            && let Some(s) = self.ss.find_syntax_by_extension(ext) {
                return Some(s);
            }
        // Fall back to first-line / content sniffing.
        self.ss.find_syntax_for_file(p).ok().flatten()
    }

    /// Highlight `content` line-by-line, returning one HTML fragment per line.
    /// Lines keep their trailing newline stripped; the caller adds gutter markup.
    pub fn highlight(&self, path: &str, content: &str, theme: &Theme) -> Result<Vec<String>> {
        let syntax = match self.syntax_for_path(path) {
            Some(s) => s,
            None => {
                // Plain text: just HTML-escape each line.
                return Ok(content.split_inclusive('\n').map(escape_html).collect());
            }
        };
        let mut hl = HighlightLines::new(syntax, theme);
        let mut out = Vec::new();
        for raw in content.split_inclusive('\n') {
            let regions = hl.highlight_line(raw, &self.ss)?;
            let html = styled_line_to_highlighted_html(&regions, IncludeBackground::No)?;
            out.push(html);
        }
        Ok(out)
    }
}

/// The page-level color scheme derived from a syntect theme.
///
/// syntect bakes *syntax token* colors into inline styles at build time, but
/// the page chrome (background, gutters, panels, borders) is driven by CSS
/// variables. To make `--theme` actually change the whole page — not just the
/// token colors — we read the theme's own background/foreground and derive the
/// rest of the palette from them, then inject them as CSS variables.
pub struct ThemeColors {
    pub bg: String,
    pub fg: String,
    pub fg_dim: String,
    pub bg_elev: String,
    pub bg_row: String,
    pub bg_hover: String,
    pub bg_cov: String,
    pub bg_pin: String,
    pub border: String,
    pub accent: String,
    pub fail: String,
    #[allow(dead_code)]
    pub is_dark: bool,
}

impl ThemeColors {
    /// Extract the page palette from a syntect theme, deriving sensible shades
    /// for elevations/hover/borders from the theme's background & foreground.
    pub fn from_theme(theme: &Theme) -> ThemeColors {
        let s = &theme.settings;
        let bg = s
            .background
            .map(rgb)
            .unwrap_or([0x1b, 0x22, 0x29]);
        let fg = s
            .foreground
            .map(rgb)
            .unwrap_or([0xc0, 0xc5, 0xce]);
        let bright = 0.299 * f32::from(bg[0]) + 0.587 * f32::from(bg[1]) + 0.114 * f32::from(bg[2]);
        let is_dark = bright < 128.0;
        // For a dark theme, lighten the bg for elevations; for a light theme,
        // darken it.
        let toward = if is_dark {
            [255u8, 255, 255]
        } else {
            [0u8, 0, 0]
        };
        let green = [0x99u8, 0xc7, 0x94];

        ThemeColors {
            bg: hex(bg),
            fg: hex(fg),
            fg_dim: hex(mix(fg, bg, 0.45)),
            bg_elev: hex(mix(bg, toward, 0.06)),
            bg_row: hex(mix(bg, toward, 0.035)),
            bg_hover: hex(mix(bg, toward, 0.13)),
            bg_cov: hex(mix(bg, green, 0.12)),
            bg_pin: hex(mix(bg, green, 0.24)),
            border: hex(mix(bg, toward, 0.16)),
            accent: if is_dark { "#8fa1b3" } else { "#2563eb" }.to_string(),
            fail: if is_dark { "#bf616a" } else { "#c0392b" }.to_string(),
            is_dark,
        }
    }

    /// Emit the palette as a `:root { ... }` CSS block (overrides the defaults
    /// in style.css when placed after the stylesheet link).
    pub fn to_css(&self) -> String {
        format!(
            ":root{{--bg:{bg};--fg:{fg};--fg-dim:{fg_dim};--bg-elev:{bg_elev};\
--bg-row:{bg_row};--bg-hover:{bg_hover};--bg-cov:{bg_cov};--bg-pin:{bg_pin};\
--border:{border};--accent:{accent};--fail:{fail};}}",
            bg = self.bg,
            fg = self.fg,
            fg_dim = self.fg_dim,
            bg_elev = self.bg_elev,
            bg_row = self.bg_row,
            bg_hover = self.bg_hover,
            bg_cov = self.bg_cov,
            bg_pin = self.bg_pin,
            border = self.border,
            accent = self.accent,
            fail = self.fail,
        )
    }
}

fn rgb(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b]
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| -> u8 {
        (f32::from(x) * (1.0 - t) + f32::from(y) * t).round().clamp(0.0, 255.0) as u8
    };
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// Escape text for HTML (used for plain-text / fallback rendering).
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}
