use crate::report::database::SourceFile;
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

/// Build `(tests_array_js, coverage_map_js)`.
fn build_data(
    tests: &[TestView<'_>],
    coverage: &BTreeMap<String, SourceFile>,
) -> (String, String) {
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
    for (i, (path, src)) in coverage.iter().enumerate() {
        if i > 0 {
            cov_js.push(',');
        }
        cov_js.push_str(&json_str(path));
        cov_js.push(':');
        cov_js.push_str(&lines_obj(&src.lines));
    }
    cov_js.push('}');
    (tests_js, cov_js)
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
/// `<tr>` attributes (e.g. `data-line`).
fn source_block<F: Fn(&str) -> String>(
    highlighted: &[String],
    cov: &BTreeMap<String, Vec<u32>>,
    tr_attrs: &F,
) -> String {
    let mut s = String::new();
    s.push_str("<pre class=\"source\"><code><table>");
    for (i, frag) in highlighted.iter().enumerate() {
        let lineno = (i + 1).to_string();
        let is_cov = cov.contains_key(&lineno);
        s.push_str("<tr");
        if is_cov {
            s.push_str(" class=\"cov\"");
        }
        s.push_str(&tr_attrs(&lineno));
        s.push('>');
        s.push_str(&format!("<td class=\"ln\">{lineno}</td>"));
        s.push_str("<td class=\"lc\">");
        s.push_str(frag);
        s.push_str("</td></tr>");
    }
    s.push_str("</table></code></pre>");
    s
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

    let (tests_js, _) = build_data(tests, coverage);

    // --- index.html ---
    {
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
        html.push_str("<ul class=\"filelist\">");
        let mut paths: Vec<&String> = coverage.keys().collect();
        paths.sort();
        for path in paths {
            let n = coverage[path].lines.len();
            html.push_str(&format!(
                "<li><a href=\"{}.html\">{name}</a> <span class=\"count\">{n} line(s)</span></li>",
                escape(path),
                name = escape(path)
            ));
        }
        html.push_str("</ul></main></body></html>");
        fs::write(out_dir.join("index.html"), html)?;
    }

    // --- per-file pages ---
    for view in views {
        let rel = std::path::Path::new(&view.path);
        let dest = out_dir.join(format!("{}.html", rel.to_string_lossy()));
        fs::create_dir_all(dest.parent().unwrap())?;

        let prefix = up_prefix(&view.path);
        let cov = coverage[&view.path].clone();
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
        html.push_str("</div>");

        html.push_str(&source_block(&view.highlighted, &cov.lines, &dir_attrs));

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
        html.push_str("};</script>");
        html.push_str(&format!("<script src=\"{prefix}js/app.js\"></script>"));
        html.push_str("</body></html>");

        fs::write(&dest, html)?;
    }

    // --- tests.html (catalog of every observed test) ---
    render_tests_page(out_dir, theme_css, tests, coverage)?;

    Ok(())
}

/// Client-side column sort for the tests catalog. Pair-aware: each failed
/// test is followed by a trailing `.failout` row holding its output, so we
/// sort the primary rows and re-thread their output rows behind them rather
/// than sorting every `<tr>` independently (which would detach outputs from
/// their tests).
const TESTS_SORT_JS: &str = "\
(function(){var t=document.querySelector('table.tests-table');if(!t)return;\
var tb=t.tBodies[0];if(!tb)return;var hs=t.querySelectorAll('th[data-sort]');\
var key=null,dir=1;hs.forEach(function(th){th.addEventListener('click',function(){\
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
pairs.forEach(function(p){tb.appendChild(p[0]);p[1].forEach(function(e){tb.appendChild(e);});});});});})();";

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

    html.push_str("<table class=\"tests-table\"><thead><tr>");
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
        // (kept attached to this test during sort — see TESTS_SORT_JS). This
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
    html.push_str(&format!("<script>{TESTS_SORT_JS}</script>"));
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

    let (tests_js, cov_js) = build_data(tests, coverage);

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
    html.push_str("<ul class=\"filelist\">");
    for view in views {
        let n = coverage[&view.path].lines.len();
        html.push_str(&format!(
            "<li><a href=\"#file-{id}\">{name}</a> <span class=\"count\">{n} line(s)</span></li>",
            id = fnv1a(&view.path),
            name = escape(&view.path)
        ));
    }
    html.push_str("</ul></main>");

    for view in views {
        let cov = &coverage[&view.path];
        let esc_file = escape(&view.path);
        let sf_attrs = move |ln: &str| format!(" data-file=\"{esc_file}\" data-line=\"{ln}\"");
        html.push_str(&format!(
            "<section class=\"file\" id=\"file-{id}\"><div class=\"toolbar\">\
             <span class=\"path\">{name}</span></div>",
            id = fnv1a(&view.path),
            name = escape(&view.path)
        ));
        html.push_str(&source_block(&view.highlighted, &cov.lines, &sf_attrs));
        html.push_str("</section>");
    }

    html.push_str("<div id=\"panel\" class=\"panel\" role=\"status\">");
    html.push_str("<span class=\"hint\">Hover a highlighted line to see covering tests · click to pin</span>");
    html.push_str("</div>");

    html.push_str("<script>window.__TESTMAP_TESTS=");
    html.push_str(&tests_js);
    html.push_str(";window.__TESTMAP_COV=");
    html.push_str(&cov_js);
    html.push_str(";</script><script>");
    html.push_str(JS);
    html.push_str("</script></body></html>");

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out_path, html)?;
    Ok(())
}
