pub mod database;
pub mod highlight;
pub mod html;

use crate::cli::ReportArgs;
use anyhow::Result;
use std::path::PathBuf;

pub fn run(args: ReportArgs) -> Result<()> {
    let input = PathBuf::from(&args.input);
    let db = database::Database::read(&input)?;

    let highlighter = highlight::Highlighter::get();
    let theme_name = args.theme.as_deref().unwrap_or("base16-ocean.dark");
    let theme = highlighter.resolve_theme(Some(theme_name))?;
    // Derive the page chrome (background/gutters/panels/…) from the chosen
    // theme so `--theme` changes the whole page, not just the syntax tokens.
    let theme_css = highlight::ThemeColors::from_theme(theme).to_css();

    // Build test views in the same order/index as the database's tests array.
    let tests: Vec<html::TestView<'_>> = db
        .tests
        .iter()
        .map(|t| html::TestView {
            name: &t.name,
            module: &t.module,
            binary: &t.binary,
            kind: &t.kind,
            status: &t.status,
            duration_ms: t.duration_ms,
        })
        .collect();

    // Highlight every source file once.
    let mut views: Vec<html::FileView> = Vec::new();
    for (path, src) in &db.sources {
        let highlighted = highlighter
            .highlight(path, &src.content, theme)
            .map_err(|e| anyhow::anyhow!("highlighting {path}: {e}"))?;
        views.push(html::FileView {
            path: path.clone(),
            highlighted,
        });
    }

    match args.single_file {
        Some(path) => {
            let out = PathBuf::from(&path);
            html::render_single_file(&out, theme_name, &theme_css, &tests, &db.sources, &views)?;
            eprintln!("✓ wrote single-file report → {}", out.display());
        }
        None => {
            let out_dir = PathBuf::from(&args.output_dir);
            html::render_directory(&out_dir, theme_name, &theme_css, &tests, &db.sources, &views)?;
            eprintln!(
                "✓ wrote report → {} (open {}/index.html)",
                out_dir.display(),
                out_dir.display()
            );
        }
    }
    Ok(())
}
