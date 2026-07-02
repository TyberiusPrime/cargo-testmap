use crate::report::classify::{self, LineClass, LineStats};
use crate::report::database::{AboveThreshold, SourceFile};
use crate::report::highlight::escape_html as escape;
use crate::util::fnv1a;
use std::collections::BTreeMap;

const CSS: &str = include_str!("style.css");
const JS: &str = include_str!("app.js");

/// A minimal test descriptor embedded into the report JS.
pub struct TestView<'a> {
    pub name: &'a str,
    pub module: &'a str,
    pub binary: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub duration_ms: u64,
    /// Captured output (stderr+stdout) of a failed test, if any.
    pub failure_output: Option<&'a str>,
}

/// Per-file highlighted source (path + one HTML fragment per line).
pub struct FileView {
    pub path: String,
    pub highlighted: Vec<String>,
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| String::from("\"\""))
}

/// Build `(tests_array_js, coverage_map_js, above_threshold_map_js)`.
fn build_data(
    tests: &[TestView<'_>],
    coverage: &BTreeMap<String, SourceFile>,
) -> (String, String, String) {
    let mut tests_js = String::from("[");
    for (i, t) in tests.iter().enumerate() {
        if i > 0 {
            tests_js.push(',');
        }
        tests_js.push_str(&format!(
            "{{n:{},m:{},b:{},k:{},s:{}}}",
            json_str(t.name),
            json_str(t.module),
            json_str(t.binary),
            json_str(t.kind),
            json_str(t.status),
        ));
    }
    tests_js.push(']');

    let mut cov_js = String::from('{');
    let mut above_js = String::from('{');
    for (i, (path, src)) in coverage.iter().enumerate() {
        if i > 0 {
            cov_js.push(',');
            above_js.push(',');
        }
        cov_js.push_str(&json_str(path));
        cov_js.push(':');
        cov_js.push_str(&lines_obj(&src.lines));
        above_js.push_str(&json_str(path));
        above_js.push(':');
        above_js.push_str(&above_obj(&src.above_threshold));
    }
    cov_js.push('}');
    above_js.push('}');
    (tests_js, cov_js, above_js)
}

fn lines_obj(lines: &BTreeMap<String, Vec<u32>>) -> String {
    let mut s = String::from('{');
    for (j, (line, idxs)) in lines.iter().enumerate() {
        if j > 0 {
            s.push(',');
        }
        s.push_str(&json_str(line));
        s.push_str(":[");
        for (k, x) in idxs.iter().enumerate() {
            if k > 0 {
                s.push(',');
            }
            s.push_str(&x.to_string());
        }
        s.push(']');
    }
    s.push('}');
    s
}

/// Like [`lines_obj`] but maps line -> `{total, sample}` (for above-threshold
/// lines, where we keep a small sample of the covering tests, not all).
fn above_obj(above: &BTreeMap<String, AboveThreshold>) -> String {
    let mut s = String::from('{');
    for (j, (line, info)) in above.iter().enumerate() {
        if j > 0 {
            s.push(',');
        }
        s.push_str(&json_str(line));
        s.push_str(&format!(":{{total:{},sample:[", info.total));
        for (k, x) in info.sample.iter().enumerate() {
            if k > 0 {
                s.push(',');
            }
            s.push_str(&x.to_string());
        }
        s.push_str("]}");
    }
    s.push('}');
    s
}

/// Number of "../" needed to get from `out_dir/<path>.html` back to `out_dir`.
fn up_prefix(path: &str) -> String {
    "../".repeat(path.matches('/').count())
}

/// For each test index, how many mapped lines it covers. Inverts the
/// (post-threshold) coverage map.
fn per_test_counts(n_tests: usize, coverage: &BTreeMap<String, SourceFile>) -> Vec<u32> {
    let mut counts = vec![0u32; n_tests];
    for src in coverage.values() {
        for idxs in src.lines.values() {
            for &i in idxs {
                if (i as usize) < n_tests {
                    counts[i as usize] += 1;
                }
            }
        }
    }
    counts
}

/// Emit the highlighted source block. `tr_attrs(lineno)` returns the extra
/// `<tr>` attributes (e.g. `data-line`). `classes[i]` classifies line `i+1`
/// and drives both the gutter dot's color (via a per-row CSS class) and a
/// `title` tooltip explaining the dot.
fn source_block<F: Fn(&str) -> String>(
    highlighted: &[String],
    classes: &[LineClass],
    tr_attrs: &F,
) -> String {
    let mut s = String::new();
    s.push_str("<pre class=\"source\"><code><table>");
    for (i, frag) in highlighted.iter().enumerate() {
        let lineno = (i + 1).to_string();
        let class = classes.get(i).copied().unwrap_or(LineClass::None);
        s.push_str("<tr");
        if let Some(c) = class.css_class() {
            s.push_str(&format!(" class=\"{c}\""));
        }
        s.push_str(&tr_attrs(&lineno));
        s.push('>');
        // Put the explanatory tooltip on the gutter cell so hovering the dot
        // (rendered via `td.ln::before`) explains the color.
        let title = class.title();
        if title.is_empty() {
            s.push_str(&format!("<td class=\"ln\">{lineno}</td>"));
        } else {
            s.push_str(&format!(
                "<td class=\"ln\" title=\"{}\">{lineno}</td>",
                escape(title)
            ));
        }
        s.push_str("<td class=\"lc\">");
        s.push_str(frag);
        s.push_str("</td></tr>");
    }
    s.push_str("</table></code></pre>");
    s
}

/// The color legend explaining the gutter dots. Rendered on the index and on
/// single-file reports.
fn legend_html() -> &'static str {
    "<div class=\"legend\">\
<span><span class=\"dot covered\"></span>covered</span> \
<span><span class=\"dot uncovered\"></span>uncovered</span> \
<span><span class=\"dot excluded\"></span>excluded</span> \
<span><span class=\"dot excl-covered\"></span>excluded but covered</span> \
<span><span class=\"dot ignored\"></span>ignored</span></div>"
}

/// How (or whether) the "N uncovered" / "N ignored" parts of a stats line
/// react to being clicked.
#[derive(Clone, Copy)]
pub enum StatsClick<'a> {
    /// Plain spans — no interaction (e.g. the aggregate index total).
    None,
    /// Clicking scrolls to the next matching line on the current page
    /// (per-file pages, and the single-file report's grand total).
    ScrollPage,
    /// Clicking scrolls to the next matching line within one file's section
    /// (single-file report's per-file rows).
    ScrollFile(&'a str),
}

/// Render a coverage summary line for a totals or per-file block.
/// `uncovered` is `coverable - covered`. The "uncovered" and "ignored" parts
/// become clickable jump links when `click` asks for it.
fn stats_html(s: LineStats, click: StatsClick<'_>) -> String {
    let uncovered = s.coverable.saturating_sub(s.covered);
    let pct = match s.pct() {
        Some(p) => format!("{p}%"),
        None => "—".to_string(),
    };
    let mut out = format!(
        "<span class=\"covered\">{}</span> / <span class=\"coverable\">{}</span>",
        s.covered, s.coverable
    );
    out.push_str(&format!(" <span class=\"pct\">({pct})</span>"));
    if uncovered > 0 {
        out.push_str(&jump_part("uncovered", uncovered, "gap", click));
    }
    if s.excluded > 0 {
        out.push_str(&jump_part("excluded", s.excluded, "muted", click));
    }
    if s.ignored > 0 {
        out.push_str(&jump_part("ignored", s.ignored, "muted", click));
    }
    out
}

/// Render one clickable-or-plain "N <kind>" fragment for [`stats_html`].
fn jump_part(kind: &str, n: u32, base: &str, click: StatsClick<'_>) -> String {
    match click {
        StatsClick::None => format!(" · <span class=\"{base}\">{n} {kind}</span>"),
        StatsClick::ScrollPage => format!(
            " · <a class=\"jump {base}\" data-jump=\"{kind}\" href=\"#\" role=\"button\" tabindex=\"0\">{n} {kind}</a>"
        ),
        StatsClick::ScrollFile(path) => format!(
            " · <a class=\"jump {base}\" data-jump=\"{kind}\" data-file-id=\"{id}\" href=\"#\" role=\"button\" tabindex=\"0\">{n} {kind}</a>",
            id = fnv1a(path)
        ),
    }
}

/// A neutral `-` placeholder for a zero count in the index table's
/// uncovered/excluded/ignored columns: reads as "none" in the default font
/// color instead of a colored `0`. The cell keeps `data-v="0"`, so the column
/// still sorts correctly.
fn zero_dash() -> String {
    "<span class=\"dash\">-</span>".to_string()
}

/// A plain count for a non-jump index column (e.g. excluded), rendered as a
/// [`zero_dash`] when zero.
fn count_or_dash(n: u32) -> String {
    if n == 0 {
        zero_dash()
    } else {
        n.to_string()
    }
}

/// A bare count cell for the index table's uncovered/ignored columns: a plain
/// `<a class="jump …">` (linking into the file page at the first matching line
/// via `?jump=`) when non-zero, or a [`zero_dash`] when there's nothing to
/// jump to.
fn jump_count_link(url: &str, kind: &str, n: u32, base: &str) -> String {
    if n == 0 {
        zero_dash()
    } else {
        format!(
            "<a class=\"jump {base}\" href=\"{}?jump={kind}\">{n}</a>",
            escape(url)
        )
    }
}

/// Render a multi-file directory report (the default).
pub fn render_directory(
    out_dir: &std::path::Path,
    theme_name: &str,
    theme_css: &str,
    tests: &[TestView<'_>],
    coverage: &BTreeMap<String, SourceFile>,
    views: &[FileView],
) -> std::io::Result<()> {
    use std::fs;

    fs::create_dir_all(out_dir.join("css"))?;
    fs::create_dir_all(out_dir.join("js"))?;
    fs::write(out_dir.join("css").join("style.css"), CSS)?;
    fs::write(out_dir.join("js").join("app.js"), JS)?;

    let (tests_js, _, _) = build_data(tests, coverage);

    // Classify every file once; reuse for both the index totals and the
    // per-file gutter dots so the work isn't duplicated.
    let class_map: BTreeMap<String, (Vec<LineClass>, LineStats)> = coverage
        .iter()
        .map(|(path, src)| {
            let classes = classify::classify(src);
            let stats = classify::stats(&classes);
            (path.clone(), (classes, stats))
        })
        .collect();

    // --- index.html ---
    {
        let mut total = LineStats::default();
        for (_, s) in class_map.values() {
            total.coverable += s.coverable;
            total.covered += s.covered;
            total.excluded += s.excluded;
            total.ignored += s.ignored;
        }

        let mut html = String::new();
        html.push_str("<!doctype html><html lang=\"en\"><head>");
        html.push_str("<meta charset=\"utf-8\">");
        html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
        html.push_str("<title>testmap report</title>");
        html.push_str("<link rel=\"stylesheet\" href=\"css/style.css\">");
        html.push_str(&format!("<style>{theme_css}</style>"));
        html.push_str("</head><body>");
        html.push_str("<main class=\"index\"><h1>testmap</h1>");
        html.push_str(&format!(
            "<p class=\"meta\">{} test(s) · {} file(s) · theme: {} · <a href=\"tests.html\">view all tests →</a></p>",
            tests.len(),
            coverage.len(),
            escape(theme_name)
        ));
        // Total coverable line count so missing coverage is spottable at a glance.
        html.push_str(&format!(
            "<p class=\"total\">total: {}</p>",
            stats_html(total, StatsClick::None)
        ));
        html.push_str(legend_html());
        // Sortable per-file table. Initial order is worst-coverage-first (then
        // most uncovered, then path) so files missing coverage jump out instead
        // of being buried alphabetically; the column headers re-sort client-side.
        let mut order: Vec<(&String, LineStats)> = class_map
            .iter()
            .map(|(p, (_, s))| (p, *s))
            .collect();
        order.sort_by(|a, b| {
            // Ascending coverage %, then most uncovered, then path.
            let pa = a.1.pct().unwrap_or(0);
            let pb = b.1.pct().unwrap_or(0);
            pa.cmp(&pb)
                .then_with(|| {
                    let ua = a.1.coverable.saturating_sub(a.1.covered);
                    let ub = b.1.coverable.saturating_sub(b.1.covered);
                    ub.cmp(&ua)
                })
                .then_with(|| a.0.cmp(b.0))
        });
        html.push_str("<table class=\"filelist-table\" data-sortable><thead><tr>");
        html.push_str("<th data-sort=\"path\">file</th>");
        html.push_str("<th data-sort=\"covered\" data-numeric=\"1\">covered</th>");
        html.push_str("<th data-sort=\"coverable\" data-numeric=\"1\">coverable</th>");
        html.push_str("<th data-sort=\"pct\" data-numeric=\"1\">%</th>");
        html.push_str("<th data-sort=\"uncovered\" data-numeric=\"1\">uncovered</th>");
        html.push_str("<th data-sort=\"excluded\" data-numeric=\"1\">excluded</th>");
        html.push_str("<th data-sort=\"ignored\" data-numeric=\"1\">ignored</th>");
        html.push_str("</tr></thead><tbody>");
        for (path, s) in order {
            let url = format!("{}.html", path);
            let uncovered = s.coverable.saturating_sub(s.covered);
            let (pct_v, pct_txt) = match s.pct() {
                Some(p) => (p, format!("{p}%")),
                None => (0, "—".to_string()),
            };
            html.push_str("<tr>");
            html.push_str(&format!(
                "<td class=\"fname\" data-v=\"{}\"><a href=\"{}\">{}</a></td>",
                escape(path),
                escape(&url),
                escape(path)
            ));
            html.push_str(&format!(
                "<td class=\"num covered\" data-v=\"{}\">{}</td>",
                s.covered, s.covered
            ));
            html.push_str(&format!(
                "<td class=\"num coverable\" data-v=\"{}\">{}</td>",
                s.coverable, s.coverable
            ));
            html.push_str(&format!(
                "<td class=\"num pct\" data-v=\"{}\">{}</td>",
                pct_v, pct_txt
            ));
            html.push_str(&format!(
                "<td class=\"num gap\" data-v=\"{}\">{}</td>",
                uncovered,
                jump_count_link(&url, "uncovered", uncovered, "gap")
            ));
            html.push_str(&format!(
                "<td class=\"num muted\" data-v=\"{}\">{}</td>",
                s.excluded,
                count_or_dash(s.excluded)
            ));
            html.push_str(&format!(
                "<td class=\"num muted\" data-v=\"{}\">{}</td>",
                s.ignored,
                jump_count_link(&url, "ignored", s.ignored, "muted")
            ));
            html.push_str("</tr>");
        }
        html.push_str("</tbody></table>");
        html.push_str(&format!("<script>{TABLE_SORT_JS}</script>"));
        html.push_str("</main></body></html>");
        fs::write(out_dir.join("index.html"), html)?;
    }

    // --- per-file pages ---
    for view in views {
        let rel = std::path::Path::new(&view.path);
        let dest = out_dir.join(format!("{}.html", rel.to_string_lossy()));
        fs::create_dir_all(dest.parent().unwrap())?;

        let prefix = up_prefix(&view.path);
        let cov = coverage[&view.path].clone();
        let classes = class_map[&view.path].0.clone();
        let dir_attrs = |ln: &str| format!(" data-line=\"{ln}\"");

        let mut html = String::new();
        html.push_str("<!doctype html><html lang=\"en\"><head>");
        html.push_str("<meta charset=\"utf-8\">");
        html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
        html.push_str(&format!("<title>{} — testmap</title>", escape(&view.path)));
        html.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"{prefix}css/style.css\">"
        ));
        html.push_str(&format!("<style>{theme_css}</style>"));
        html.push_str("</head><body>");

        html.push_str("<div class=\"toolbar\">");
        html.push_str(&format!("<a class=\"back\" href=\"{prefix}index.html\">← index</a>"));
        html.push_str(&format!("<span class=\"path\">{}</span>", escape(&view.path)));
        html.push_str(&format!(
            "<span class=\"toolbar-stats\">{}</span>",
            stats_html(class_map[&view.path].1, StatsClick::ScrollPage)
        ));
        html.push_str("</div>");

        html.push_str(&source_block(&view.highlighted, &classes, &dir_attrs));

        html.push_str("<div id=\"panel\" class=\"panel\" role=\"status\">");
        html.push_str("<span class=\"hint\">Hover a highlighted line to see covering tests · click to pin</span>");
        html.push_str("</div>");

        // Embed tests + this file's coverage only.
        html.push_str("<script>window.__TESTMAP_TESTS=");
        html.push_str(&tests_js);
        html.push_str(";window.__TESTMAP_FILE=");
        html.push_str(&json_str(&view.path));
        html.push_str(";window.__TESTMAP_COV={");
        html.push_str(&json_str(&view.path));
        html.push(':');
        html.push_str(&lines_obj(&cov.lines));
        html.push_str("};window.__TESTMAP_ABOVE={");
        html.push_str(&json_str(&view.path));
        html.push(':');
        html.push_str(&above_obj(&cov.above_threshold));
        html.push_str("};</script>");
        html.push_str(&format!("<script src=\"{prefix}js/app.js\"></script>"));
        html.push_str("</body></html>");

        fs::write(&dest, html)?;
    }

    // --- tests.html (catalog of every observed test) ---
    render_tests_page(out_dir, theme_css, tests, coverage)?;

    Ok(())
}

/// Client-side column sort for any `<table data-sortable>`. Click a `th` with a
/// `data-sort` key to sort; numeric columns carry `data-numeric="1"`. Each
/// cell's sort value comes from its `data-v`, falling back to trimmed text.
/// Pair-aware: a primary row may be followed by trailing `.failout` rows
/// (failed-test output on the tests catalog) that must stay attached to their
/// test, so we sort (row, extras) pairs rather than every `<tr>` independently
/// (which would detach outputs from their tests).
const TABLE_SORT_JS: &str = "\
(function(){var ts=document.querySelectorAll('table[data-sortable]');\
ts.forEach(function(t){var tb=t.tBodies[0];if(!tb)return;\
var hs=t.querySelectorAll('th[data-sort]');var key=null,dir=1;\
hs.forEach(function(th){th.addEventListener('click',function(){\
var k=th.dataset.sort,num=th.dataset.numeric==='1';dir=(key===k)?-dir:1;key=k;\
hs.forEach(function(h){h.classList.remove('sort-asc','sort-desc');});\
th.classList.add(dir>0?'sort-asc':'sort-desc');var c=th.cellIndex;\
var rows=Array.prototype.slice.call(tb.rows);var pairs=[];\
for(var i=0;i<rows.length;){var main=rows[i++];var extras=[];\
while(i<rows.length&&rows[i].classList.contains('failout')){extras.push(rows[i++]);}\
pairs.push([main,extras]);}\
pairs.sort(function(a,b){var ra=a[0],rb=b[0];\
var av=ra.cells[c].dataset.v;if(av===undefined)av=ra.cells[c].textContent.trim();\
var bv=rb.cells[c].dataset.v;if(bv===undefined)bv=rb.cells[c].textContent.trim();\
if(num){av=+av;bv=+bv;}return av<bv?-dir:av>bv?dir:0;});\
while(tb.firstChild)tb.removeChild(tb.firstChild);\
pairs.forEach(function(p){tb.appendChild(p[0]);p[1].forEach(function(e){tb.appendChild(e);});});});});});})();";

/// Render `tests.html` — a catalog of every test testmap observed, with the
/// number of mapped lines each one covers. Tests that ran but covered no
/// snapshotted code (0 lines) are highlighted so they jump out when trying to
/// understand coverage gaps.
pub fn render_tests_page(
    out_dir: &std::path::Path,
    theme_css: &str,
    tests: &[TestView<'_>],
    coverage: &BTreeMap<String, SourceFile>,
) -> std::io::Result<()> {
    use std::fs;

    let counts = per_test_counts(tests.len(), coverage);
    let zero = counts.iter().filter(|&&c| c == 0).count();
    let nonzero = tests.len().saturating_sub(zero);
    let failed = tests.iter().filter(|t| t.status == "failed").count();

    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head>");
    html.push_str("<meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>Observed tests — testmap</title>");
    html.push_str("<link rel=\"stylesheet\" href=\"css/style.css\">");
    html.push_str(&format!("<style>{theme_css}</style>"));
    html.push_str("</head><body>");

    html.push_str("<div class=\"toolbar\">");
    html.push_str("<a class=\"back\" href=\"index.html\">← index</a>");
    html.push_str(&format!(
        "<span class=\"path\">Observed tests ({})</span>",
        tests.len()
    ));
    html.push_str("</div>");

    html.push_str("<main class=\"tests-page\">");
    // Summary line. The failed count is always shown so the user can tell at a
    // glance whether anything broke; it's highlighted only when non-zero.
    let mut meta = format!("<p class=\"meta\">{} test(s) observed · ", tests.len());
    meta.push_str(&if failed > 0 {
        format!("<span class=\"failcount\">{} failed</span> · ", failed)
    } else {
        format!("<span class=\"muted\">{} failed</span> · ", failed)
    });
    meta.push_str(&format!(
        "{} cover ≥1 mapped line · \
         <span class=\"zerocount\">{} cover 0 mapped lines</span> \
         <span class=\"muted\">(ran but exercised no snapshotted code — \
         often filtered, skipped, or broken)</span></p>",
        nonzero,
        zero
    ));
    html.push_str(&meta);

    html.push_str("<table class=\"tests-table\" data-sortable><thead><tr>");
    html.push_str("<th data-sort=\"idx\">#</th>");
    html.push_str("<th data-sort=\"status\">status</th>");
    html.push_str("<th data-sort=\"kind\">kind</th>");
    html.push_str("<th data-sort=\"binary\">binary</th>");
    html.push_str("<th data-sort=\"path\">test</th>");
    html.push_str("<th data-sort=\"lines\" data-numeric=\"1\">lines</th>");
    html.push_str("<th data-sort=\"dur\" data-numeric=\"1\">dur&nbsp;ms</th>");
    html.push_str("</tr></thead><tbody>");

    for (i, t) in tests.iter().enumerate() {
        let path = if t.module.is_empty() {
            t.name.to_string()
        } else {
            format!("{}::{}", t.module, t.name)
        };
        let failed = t.status == "failed";
        let cls = if failed {
            "failed"
        } else if counts[i] == 0 {
            "zero"
        } else {
            ""
        };
        let status_v = if failed { "1" } else { "0" };
        html.push_str(&format!("<tr class=\"{cls}\">"));
        html.push_str(&format!("<td data-v=\"{i}\">{i}</td>"));
        html.push_str(&format!(
            "<td data-v=\"{status_v}\">{}</td>",
            t.status
        ));
        html.push_str(&format!(
            "<td data-v=\"{}\"><span class=\"badge\">{}</span></td>",
            escape(t.kind),
            escape(t.kind)
        ));
        html.push_str(&format!(
            "<td data-v=\"{}\"><span class=\"badge\">{}</span></td>",
            escape(t.binary),
            escape(t.binary)
        ));
        html.push_str(&format!(
            "<td class=\"tname\" data-v=\"{}\">{}</td>",
            escape(&path),
            escape(&path)
        ));
        html.push_str(&format!(
            "<td class=\"num\" data-v=\"{}\">{}</td>",
            counts[i],
            counts[i]
        ));
        html.push_str(&format!(
            "<td class=\"num\" data-v=\"{}\">{}</td>",
            t.duration_ms,
            t.duration_ms
        ));
        html.push_str("</tr>");
        // A failed test's captured output lives in a trailing full-width row
        // (kept attached to this test during sort — see TABLE_SORT_JS). This
        // is the report's way of answering "why did this test fail?".
        if failed {
            match t.failure_output.filter(|s| !s.trim().is_empty()) {
                Some(out) => {
                    html.push_str(&format!(
                        "<tr class=\"failout\"><td colspan=\"7\">\
                         <details><summary>output</summary>\
                         <pre class=\"failout-pre\">{}</pre></details></td></tr>",
                        escape(out)
                    ));
                }
                None => {
                    html.push_str(
                        "<tr class=\"failout\"><td colspan=\"7\">\
                         <span class=\"muted\">(no output captured)</span></td></tr>",
                    );
                }
            }
        }
    }
    html.push_str("</tbody></table>");
    html.push_str("</main>");
    html.push_str(&format!("<script>{TABLE_SORT_JS}</script>"));
    html.push_str("</body></html>");

    fs::write(out_dir.join("tests.html"), html)?;
    Ok(())
}
pub fn render_single_file(
    out_path: &std::path::Path,
    theme_name: &str,
    theme_css: &str,
    tests: &[TestView<'_>],
    coverage: &BTreeMap<String, SourceFile>,
    views: &[FileView],
) -> std::io::Result<()> {
    use std::fs;

    let (tests_js, cov_js, above_js) = build_data(tests, coverage);

    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head>");
    html.push_str("<meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>testmap report</title><style>");
    html.push_str(CSS);
    html.push_str(theme_css);
    html.push_str("</style></head><body>");

    html.push_str("<main class=\"index\"><h1>testmap</h1>");
    html.push_str(&format!(
        "<p class=\"meta\">{} test(s) · {} file(s) · theme: {}</p>",
        tests.len(),
        coverage.len(),
        escape(theme_name)
    ));
    // Total coverable count + legend (same info as the directory index).
    let mut total = LineStats::default();
    let mut file_classes: BTreeMap<String, Vec<LineClass>> = BTreeMap::new();
    for (path, src) in coverage {
        let classes = classify::classify(src);
        let s = classify::stats(&classes);
        total.coverable += s.coverable;
        total.covered += s.covered;
        total.excluded += s.excluded;
        total.ignored += s.ignored;
        file_classes.insert(path.clone(), classes);
    }
    html.push_str(&format!("<p class=\"total\">total: {}</p>", stats_html(total, StatsClick::ScrollPage)));
    html.push_str(legend_html());
    html.push_str("<ul class=\"filelist\">");
    for view in views {
        let classes = &file_classes[&view.path];
        let stats = classify::stats(classes);
        html.push_str(&format!(
            "<li><a href=\"#file-{id}\">{name}</a> <span class=\"stats\">{stats}</span></li>",
            id = fnv1a(&view.path),
            name = escape(&view.path),
            stats = stats_html(stats, StatsClick::ScrollFile(view.path.as_str()))
        ));
    }
    html.push_str("</ul></main>");

    for view in views {
        let classes = &file_classes[&view.path];
        let esc_file = escape(&view.path);
        let sf_attrs = move |ln: &str| format!(" data-file=\"{esc_file}\" data-line=\"{ln}\"");
        html.push_str(&format!(
            "<section class=\"file\" id=\"file-{id}\"><div class=\"toolbar\">\
             <span class=\"path\">{name}</span></div>",
            id = fnv1a(&view.path),
            name = escape(&view.path)
        ));
        html.push_str(&source_block(&view.highlighted, classes, &sf_attrs));
        html.push_str("</section>");
    }

    html.push_str("<div id=\"panel\" class=\"panel\" role=\"status\">");
    html.push_str("<span class=\"hint\">Hover a highlighted line to see covering tests · click to pin</span>");
    html.push_str("</div>");

    html.push_str("<script>window.__TESTMAP_TESTS=");
    html.push_str(&tests_js);
    html.push_str(";window.__TESTMAP_COV=");
    html.push_str(&cov_js);
    html.push_str(";window.__TESTMAP_ABOVE=");
    html.push_str(&above_js);
    html.push_str(";</script><script>");
    html.push_str(JS);
    html.push_str("</script></body></html>");

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out_path, html)?;
    Ok(())
}
