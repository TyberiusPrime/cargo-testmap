use anyhow::Result;
use std::path::Path;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
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

    /// Resolve a theme name to an actual theme, with a sensible default.
    pub fn theme(&self, name: Option<&str>) -> &Theme {
        let default = "base16-ocean.dark";
        let name = name.unwrap_or(default);
        self.themes
            .themes
            .get(name)
            .unwrap_or_else(|| &self.themes.themes[default])
    }

    #[allow(dead_code)]
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
