//! Line classification for the report.
//!
//! Each source line is classified into one category, used both to color the
//! gutter dot and to compute per-file / total coverage in the index.
//!
//! The classification is derived purely from the database — source `content`,
//! the executable-line set, and the covered line sets — so re-running the
//! report after tweaking these rules needs no re-collection.
//!
//! ## Categories
//!
//! A line is **excluded** when coverage is explicitly *not expected*:
//!   - inside a `//cov:excl-start` … `//cov:excl-stop` region,
//!   - tagged with `//cov:excl-line`, or
//!   - containing `unreachable!` (automatic).
//!
//! A line is **ignored** when it is coverage noise we don't want muddying the
//! numbers:
//!   - inside a `//cov:ignore-start` … `//cov:ignore-stop` region,
//!   - tagged with `//cov:ignore-line`,
//!   - a panic site: `panic!`, `.unwrap()`, `.expect(`, `todo!`,
//!     `unimplemented!`, or
//!   - a multi-line macro invocation head (`matches!(`, `format!(`, `write!(`,
//!     `println!(`, …) whose uncovered status is an llvm-cov instrumentation
//!     artifact, not a real gap (see [`compute_macro_artifacts`]).
//!
//! Everything else executable is either **covered** (some test reached it) or
//! **uncovered** (a real coverage gap). Non-executable lines (blanks,
//! comments, bare braces) carry no dot and aren't counted.
//!
//! ## Multi-line macro heads (llvm-cov artifact)
//!
//! rustc source-based coverage (`-Cinstrument-coverage` → llvm-cov) anchors the
//! region for a macro's *own* generated tokens at the macro's opening line
//! (`name!(`) with a counter that is structurally always 0, while the macro's
//! arguments keep their own call-site spans and are correctly marked covered.
//! A multi-line macro therefore shows up as a single executable-but-uncovered
//! line — its head — flanked by covered argument lines: pure instrumentation
//! noise that would otherwise drag down the coverage percentage. We detect it
//! (line opens a macro whose body spills onto following lines, is uncovered,
//! yet its very next line is covered) and classify it as ignored so it is
//! neither counted nor shown red. A genuinely-unhit multi-line macro (e.g. an
//! untaken error branch) is left alone: there the following lines are also
//! uncovered, so it stays a real gap.
//!
//! Dot colors (see style.css / ThemeColors):
//!   - covered            → green
//!   - covered by 1 test  → orange       (a single point of failure)
//!   - uncovered          → red
//!   - excluded           → white        (pink if covered anyway)
//!   - ignored            → grey
//!
//! Lines inside a `#[test]` fn body are excluded from the "covered by 1 test"
//! category: a test trivially covers its own body, so flagging it would drown
//! the signal in noise.

use crate::report::database::SourceFile;
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineClass {
    /// Not executable (blank/comment/etc.) — no dot, not counted.
    None,
    /// Executable and reached by ≥1 test.
    Covered,
    /// Executable and reached by *exactly one* test — a coverage single
    /// point of failure: removing that one test loses this line's coverage.
    /// A subtype of [`LineClass::Covered`] shown with its own (orange) dot so
    /// such lines are spottable at a glance; the dual of the per-test "lines
    /// uniquely covered" count in the tests catalog.
    CoveredUnique,
    /// Executable, not excluded/ignored, not reached by any test — a gap.
    Uncovered,
    /// Explicitly excluded (`excl-*` / `unreachable!`) and *not* covered.
    Excluded,
    /// Excluded but covered anyway — usually a stale exclusion marker.
    ExcludedCovered,
    /// Ignored as coverage noise: a panic site (`ignore-*` / `.unwrap()` /
    /// `panic!` / …) or a multi-line macro invocation head whose uncovered
    /// status is an llvm-cov artifact, not a real gap.
    Ignored,
}

impl LineClass {
    /// The `<tr>` CSS class used to color this line's gutter dot.
    pub fn css_class(self) -> Option<&'static str> {
        match self {
            LineClass::None => None,
            LineClass::Covered => Some("cov-covered"),
            LineClass::CoveredUnique => Some("cov-unique"),
            LineClass::Uncovered => Some("cov-uncovered"),
            LineClass::Excluded => Some("cov-excluded"),
            LineClass::ExcludedCovered => Some("cov-excl-covered"),
            LineClass::Ignored => Some("cov-ignored"),
        }
    }

    /// Hover tooltip text for the line's gutter dot.
    pub fn title(self) -> &'static str {
        match self {
            LineClass::None => "",
            LineClass::Covered => "covered — reached by at least one test",
            LineClass::CoveredUnique => {
                "covered by exactly one test — removing it loses this line's coverage"
            }
            LineClass::Uncovered => "uncovered — no test reached this line",
            LineClass::Excluded => "excluded — coverage not expected (excl marker / unreachable!)",
            LineClass::ExcludedCovered => {
                "excluded but covered anyway — the exclusion marker may be stale"
            }
            LineClass::Ignored => {
                "ignored — panic site, ignore marker, or multi-line macro head (llvm-cov coverage artifact)"
            }
        }
    }
}

/// Aggregate counts for one file (or a total across files).
#[derive(Default, Clone, Copy)]
pub struct LineStats {
    /// Executable lines that aren't excluded or ignored — the denominator.
    pub coverable: u32,
    /// Coverable lines that were actually reached by a test.
    pub covered: u32,
    /// Covered lines reached by *exactly one* test (a single point of
    /// failure). A subset of `covered`; the report surfaces these so coverage
    /// that hinges on a lone test is visible, and so tests covering *zero*
    /// unique lines stand out as redundancy candidates.
    pub unique: u32,
    /// Executable lines marked excluded and *not* covered (white dot).
    pub excluded: u32,
    /// Executable lines marked excluded that were covered anyway — usually a
    /// stale exclusion marker (pink dot). A subset of all excluded lines;
    /// `excluded + excluded_covered` is the total excluded line count.
    pub excluded_covered: u32,
    /// Executable lines marked ignored.
    pub ignored: u32,
}

impl LineStats {
    /// Coverage percentage of coverable lines, rounded down. `None` when there
    /// is nothing coverable (so the index can render "—" instead of "0%").
    pub fn pct(self) -> Option<u32> {
        if self.coverable == 0 {
            None
        } else {
            Some(self.covered * 100 / self.coverable)
        }
    }
}

/// Classify every line of a file. Returns one [`LineClass`] per line, indexed
/// such that `classes[i]` describes line `i + 1`.
pub fn classify(src: &SourceFile) -> Vec<LineClass> {
    let (excluded, ignored) = compute_excl_ignored(&src.content);
    let test_fn_lines = compute_test_fn_lines(&src.content);
    let executable: BTreeSet<u32> = src.executable.iter().copied().collect();
    let covered: BTreeSet<u32> = src
        .lines
        .keys()
        .chain(src.above_threshold.keys())
        .filter_map(|k| k.parse::<u32>().ok())
        .collect();
    let macro_artifact = compute_macro_artifacts(&src.content, &executable, &covered);

    let n = excluded.len().max(ignored.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lineno = (i + 1) as u32;
        let is_exec = executable.contains(&lineno);
        if !is_exec {
            out.push(LineClass::None);
            continue;
        }
        let excl = excluded[i];
        // A multi-line macro head phantom is the same kind of coverage noise as
        // a panic site — fold it into the ignored flag.
        let ign = ignored[i] || macro_artifact[i];
        let cov = covered.contains(&lineno);
        // "Unique" = covered by exactly one test. Such a line sits in the
        // below-threshold map with a one-element test list; an above-threshold
        // line (≥ threshold tests) can never be unique. Lines inside a
        // `#[test]` fn body are excluded — a test always covers its own body,
        // so flagging it would drown the signal in noise.
        let unique = cov && uniquely_covered(src, lineno) && !test_fn_lines[i];
        let class = if excl && cov {
            LineClass::ExcludedCovered
        } else if excl {
            LineClass::Excluded
        } else if ign {
            LineClass::Ignored
        } else if unique {
            LineClass::CoveredUnique
        } else if cov {
            LineClass::Covered
        } else {
            LineClass::Uncovered
        };
        out.push(class);
    }
    out
}

/// True if `lineno` is covered by exactly one test (a coverage single point
/// of failure: removing that one test loses the line). Such a line lives in
/// the below-threshold map with a one-element test list; an above-threshold
/// line is covered by ≥ threshold tests and is therefore never unique.
fn uniquely_covered(src: &SourceFile, lineno: u32) -> bool {
    src.lines
        .get(&lineno.to_string())
        .is_some_and(|v| v.len() == 1)
}

/// Per-line (0-indexed) flags marking lines inside a `#[test]` (or `…::test`)
/// function body — from the attribute line through the body's closing brace.
/// Such lines are trivially "covered by one test" (the test itself), so the
/// classifier excludes them from the "unique" (single-point-of-failure)
/// category; otherwise every test function's body would light up orange.
///
/// Detection is a pragmatic, literal/comment-aware brace scan, not a full
/// parser: it strips string and char literals and `//` comments before
/// counting braces, so a `}` in a format string or trailing comment can't end
/// a body early. Raw strings with hashes (`r#"…"#`) and multi-line block
/// comments aren't special-cased — rare in test functions, and the only
/// failure mode is under-marking (a test line stays covered-green instead of
/// being excluded), never a false "unique".
pub(crate) fn compute_test_fn_lines(content: &str) -> Vec<bool> {
    let lines: Vec<&str> = content.lines().collect();
    let cleaned: Vec<String> = lines.iter().map(|l| clean_for_braces(l)).collect();
    let mut out = vec![false; cleaned.len()];
    let mut i = 0;
    while i < cleaned.len() {
        if TEST_ATTR.is_match(&cleaned[i])
            && let Some(close) = find_test_body(&cleaned, i)
        {
            out[i..=close].fill(true);
            i = close + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// From a test-attribute line `start`, scan the cleaned lines for the function
/// body (a `fn` keyword, then the body's braces) and return the 0-based index
/// of the line holding the matching `}`. Returns `None` if no body is found
/// (e.g. an orphaned attribute) so the caller won't over-exclude.
fn find_test_body(cleaned: &[String], start: usize) -> Option<usize> {
    let mut saw_fn = false;
    let mut depth: i32 = 0;
    let mut open_line: Option<usize> = None;
    for (j, cl) in cleaned.iter().enumerate().skip(start) {
        if !saw_fn && FN_KW.is_match(cl) {
            saw_fn = true;
        }
        for ch in cl.chars() {
            match ch {
                '{' => {
                    if depth == 0 && saw_fn && open_line.is_none() {
                        open_line = Some(j);
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 && open_line.is_some() {
                            return Some(j);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Strip string/char literals and the trailing `//` comment from a line so the
/// remainder is safe to brace-count. Order matters: literals first (so a `//`
/// inside a string isn't mistaken for a comment), then the line comment.
fn clean_for_braces(line: &str) -> String {
    let s = STR_LIT.replace_all(line, "").into_owned();
    let s = CHAR_LIT.replace_all(&s, "").into_owned();
    match s.find("//") {
        Some(idx) => s[..idx].to_string(),
        None => s,
    }
}

static TEST_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\[[^\]]*\btest\b[^\]]*\]").unwrap());
static FN_KW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bfn\b").unwrap());
static STR_LIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:br|b|r)?"(?:\\.|[^"\\])*""#).unwrap());
static CHAR_LIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"'(?:\\.|[^'\\])*'"#).unwrap());

/// Tally a file's classification vector into [`LineStats`].
pub fn stats(classes: &[LineClass]) -> LineStats {
    let mut s = LineStats::default();
    for &c in classes {
        match c {
            LineClass::Covered => {
                s.coverable += 1;
                s.covered += 1;
            }
            LineClass::CoveredUnique => {
                s.coverable += 1;
                s.covered += 1;
                s.unique += 1;
            }
            LineClass::Uncovered => s.coverable += 1,
            LineClass::Excluded => s.excluded += 1,
            LineClass::ExcludedCovered => s.excluded_covered += 1,
            LineClass::Ignored => s.ignored += 1,
            LineClass::None => {}
        }
    }
    s
}

/// Scan source text for the exclusion/ignore markers (range and single-line)
/// plus the automatic `unreachable!` / panic detections.
///
/// Returns `(excluded, ignored)` as per-line (0-indexed) flags. The flags are
/// computed for *every* line (executable or not); callers only consult them
/// for executable lines. A line carrying a cov marker is flagged by its marker
/// family (`excl-*` → excluded, `ignore-*` → ignored), so a marker comment that
/// llvm-cov spuriously instruments still reads as excluded/ignored rather than
/// as an uncovered gap.
fn compute_excl_ignored(content: &str) -> (Vec<bool>, Vec<bool>) {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut excluded = vec![false; n];
    let mut ignored = vec![false; n];
    let mut excl_on = false;
    let mut ign_on = false;

    for (i, raw) in lines.iter().enumerate() {
        let marker = line_marker(raw);

        // A line carrying a cov marker is itself a coverage-control comment,
        // not code — so even when llvm-cov spuriously instruments it (e.g. a
        // `//cov:excl-start` that a neighbouring `format!` macro bleeds a
        // counter onto), it must not read as an uncovered gap. Classify it by
        // its marker family. Every other line takes the surrounding region's
        // state.
        match marker {
            Some(Marker::ExclStart | Marker::ExclStop | Marker::ExclLine) => {
                excluded[i] = true;
            }
            Some(Marker::IgnStart | Marker::IgnStop | Marker::IgnLine) => {
                ignored[i] = true;
            }
            None => {
                if excl_on {
                    excluded[i] = true;
                }
                if ign_on {
                    ignored[i] = true;
                }
            }
        }
        // Automatic classifications (only meaningful on executable lines).
        if raw.contains("unreachable!") {
            excluded[i] = true;
        }
        if is_panic_line(raw) {
            ignored[i] = true;
        }

        // Advance region state for subsequent lines. A line carries at most
        // one toggle (see [`line_marker`]), so it can never both open and
        // close a region — which previously let trailing commentary that merely
        // *mentioned* the opposite marker (e.g. `//cov:excl-start … cov:excl-stop`)
        // collapse a region on a single line.
        match marker {
            Some(Marker::ExclStart) => excl_on = true,
            Some(Marker::ExclStop) => excl_on = false,
            Some(Marker::IgnStart) => ign_on = true,
            Some(Marker::IgnStop) => ign_on = false,
            _ => {}
        }
    }
    (excluded, ignored)
}

/// The one coverage marker a line carries, if any.
///
/// The six markers (`cov:excl-{start,stop,line}`, `cov:ignore-{start,stop,line}`)
/// are matched by plain substring, but a line is assigned **at most one**
/// role: whichever marker token appears *earliest* on the line. This makes the
/// detection robust to commentary that happens to mention another marker —
/// e.g. `//cov:excl-start until cov:excl-stop below` is an *excl-start*, not a
/// line that both opens and immediately closes the region. The token the user
/// actually typed sits at the start of the comment, so earliest-wins reliably
/// picks it over any later mention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Marker {
    ExclStart,
    ExclStop,
    ExclLine,
    IgnStart,
    IgnStop,
    IgnLine,
}

fn line_marker(line: &str) -> Option<Marker> {
    [
        ("cov:excl-start", Marker::ExclStart),
        ("cov:excl-stop", Marker::ExclStop),
        ("cov:excl-line", Marker::ExclLine),
        ("cov:ignore-start", Marker::IgnStart),
        ("cov:ignore-stop", Marker::IgnStop),
        ("cov:ignore-line", Marker::IgnLine),
    ]
    .into_iter()
    .filter_map(|(tok, m)| line.find(tok).map(|pos| (pos, m)))
    .min_by_key(|(pos, _)| *pos)
    .map(|(_, m)| m)
}

/// Recognise a panic-shaped line. `unreachable!` is deliberately *not*
/// included here — it is classified as excluded, not ignored.
fn is_panic_line(line: &str) -> bool {
    line.contains("panic!")
        || line.contains(".unwrap()")
        || line.contains(".expect(")
        || line.contains("todo!")
        || line.contains("unimplemented!")
}

/// Per-line flags marking phantom coverage gaps left by multi-line macro
/// invocations (see the module docs for *why* these exist).
///
/// A line is flagged iff it is executable and uncovered, it opens a macro
/// invocation that spills onto the following line(s) ([`opens_multiline_macro`]),
/// and the line immediately after it *is* covered — i.e. the macro body really
/// was reached, so the uncovered head is instrumentation noise rather than a
/// real gap. That last condition is what stops a genuinely-unhit multi-line
/// macro (its body lines are also uncovered) from being suppressed.
fn compute_macro_artifacts(
    content: &str,
    executable: &BTreeSet<u32>,
    covered: &BTreeSet<u32>,
) -> Vec<bool> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = vec![false; lines.len()];
    for (i, raw) in lines.iter().enumerate() {
        let lineno = (i + 1) as u32;
        // Only executable-but-uncovered lines can be the artifact.
        if !executable.contains(&lineno) || covered.contains(&lineno) {
            continue;
        }
        if !opens_multiline_macro(raw) {
            continue;
        }
        // The macro's first argument line is covered → the call was reached,
        // so the head being uncovered is the llvm-cov artifact.
        if covered.contains(&(lineno + 1)) {
            out[i] = true;
        }
    }
    out
}

/// True if `line` opens a macro invocation whose body continues onto the
/// following line(s): it contains an `ident!` immediately followed by an
/// opening delimiter, and that delimiter is left open at the end of the line.
///
/// The bracket balance is counted on the comment-stripped line. Macro *head*
/// lines (`matches!(`, `write!(`, `errors.push(format!(`, …) carry no string
/// literals, so a plain character count is reliable there; stripping `//`
/// comments keeps a stray `foo!(` in a trailing comment from false-triggering.
fn opens_multiline_macro(line: &str) -> bool {
    static MACRO_HEAD: LazyLock<Regex> = LazyLock::new(|| {
        // `ident!` then optional whitespace then an opening bracket.
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*!\s*[(\[{]").unwrap()
    });
    let code = line.split("//").next().unwrap_or(line);
    if !MACRO_HEAD.is_match(code) {
        return false;
    }
    let mut depth: i32 = 0;
    for c in code.chars() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::database::SourceFile;
    use std::collections::BTreeMap;

    /// Build a SourceFile whose `executable` set is every line (so the marker
    /// rules are the only thing distinguishing lines), with the given content
    /// and no covered lines.
    fn file(content: &str) -> SourceFile {
        let n = content.lines().count() as u32;
        SourceFile {
            content: content.to_string(),
            lines: BTreeMap::new(),
            above_threshold: BTreeMap::new(),
            executable: (1..=n).collect(),
        }
    }

    fn classes(src: &SourceFile) -> Vec<LineClass> {
        classify(src)
    }

    /// Like [`file`] but also marks the given 1-indexed lines as covered
    /// (with an empty covering-test list — presence in `lines` is all
    /// `classify` checks).
    fn covered(mut src: SourceFile, lines: &[u32]) -> SourceFile {
        for &ln in lines {
            src.lines.insert(ln.to_string(), vec![]);
        }
        src
    }

    #[test]
    fn plain_uncovered_lines_are_uncovered() {
        // No coverage, no markers → every line uncovered.
        let src = file("let a = 1;\nlet b = 2;\n");
        assert_eq!(
            classes(&src),
            vec![LineClass::Uncovered, LineClass::Uncovered]
        );
    }

    #[test]
    fn excl_line_marker_excludes() {
        let src = file("let a = 1; //cov:excl-line\nlet b = 2;\n");
        assert_eq!(classes(&src)[0], LineClass::Excluded);
        assert_eq!(classes(&src)[1], LineClass::Uncovered);
    }

    #[test]
    fn excl_start_commentary_does_not_collapse_region() {
        // Regression: a `//cov:excl-start` line whose trailing commentary
        // mentions the stop marker must NOT also close the region. Previously
        // the substring match treated the line as both start and stop, opening
        // and immediately closing the region — so the body stayed uncovered and
        // the real `-stop` became a no-op.
        let src = file(
            "let a = 1;\n\
             //cov:excl-start until cov:excl-stop below\n\
             let b = 2;\n\
             //cov:excl-stop\n\
             let d = 4;\n",
        );
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Uncovered);
        assert_eq!(c[2], LineClass::Excluded); // inside the region
        assert_eq!(c[4], LineClass::Uncovered);
    }

    #[test]
    fn excl_stop_commentary_does_not_reopen_region() {
        // Symmetric to the above: a `//cov:excl-stop` line mentioning the start
        // marker must not re-open the region after closing it.
        let src = file(
            "//cov:excl-start\n\
             let b = 2;\n\
             //cov:excl-stop pairs with cov:excl-start\n\
             let d = 4;\n",
        );
        let c = classes(&src);
        assert_eq!(c[1], LineClass::Excluded);
        assert_eq!(c[3], LineClass::Uncovered);
    }

    #[test]
    fn excl_line_commentary_mentioning_start_is_still_a_line_marker() {
        // An excl-line whose commentary mentions excl-start should stay a
        // single-line marker, not open a region.
        let src = file(
            "let a = 1; //cov:excl-line (not an excl-start)\n\
             let b = 2;\n",
        );
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Excluded);
        assert_eq!(c[1], LineClass::Uncovered);
    }

    #[test]
    fn excl_start_with_plain_commentary_still_works() {
        // Harmless trailing commentary (no other marker word) was never broken —
        // guard it against future regressions.
        let src = file(
            "//cov:excl-start reason here\n\
             let b = 2;\n\
             //cov:excl-stop\n",
        );
        assert_eq!(classes(&src)[1], LineClass::Excluded);
    }

    #[test]
    fn excl_start_stop_region_excludes_inside() {
        let src = file(
            "let a = 1;\n//cov:excl-start\nlet b = 2;\nlet c = 3;\n//cov:excl-stop\nlet d = 4;\n",
        );
        let c = classes(&src);
        // line 1: uncovered; line 2: excl-start marker → Excluded (a marker
        // line is itself a coverage-control comment, so even when llvm-cov
        // spuriously instruments it it must not read as a gap); lines 3-4:
        // excluded; line 5: excl-stop marker → Excluded; line 6: uncovered.
        assert_eq!(c[0], LineClass::Uncovered);
        assert_eq!(c[1], LineClass::Excluded); // //cov:excl-start
        assert_eq!(c[2], LineClass::Excluded);
        assert_eq!(c[3], LineClass::Excluded);
        assert_eq!(c[4], LineClass::Excluded); // //cov:excl-stop
        assert_eq!(c[5], LineClass::Uncovered);
    }

    #[test]
    fn executable_marker_line_is_not_a_gap() {
        // Regression (real-world): llvm-cov sometimes marks a `//cov:excl-start`
        // / `//cov:excl-stop` *comment* line as executable — typically a nearby
        // `format!(` macro bleeds a counter onto it. Such a marker line must
        // still be classified Excluded/Ignored, never Uncovered, otherwise the
        // very line that opts out of coverage shows up red. Here every line is
        // executable (the `file()` helper), emulating that spill.
        let src = file(
            "//cov:excl-start\n\
             errors.push(format!(\n\
             \"boom\"\n\
             ));\n\
             //cov:excl-stop\n",
        );
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Excluded); // excl-start marker
        assert_eq!(c[1], LineClass::Excluded); // errors.push(format!(
        assert_eq!(c[2], LineClass::Excluded); // "boom"
        assert_eq!(c[3], LineClass::Excluded); // ));
        assert_eq!(c[4], LineClass::Excluded); // excl-stop marker
    }

    #[test]
    fn executable_ignore_marker_line_is_ignored() {
        // Same spill, but for ignore markers → Ignored (grey), not a gap.
        let src = file("//cov:ignore-start\nlet b = 2;\n//cov:ignore-stop\n");
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Ignored);
        assert_eq!(c[1], LineClass::Ignored);
        assert_eq!(c[2], LineClass::Ignored);
    }

    #[test]
    fn unreachable_is_excluded_not_ignored() {
        let src = file("unreachable!();\n");
        assert_eq!(classes(&src)[0], LineClass::Excluded);
    }

    #[test]
    fn panics_are_ignored() {
        for body in [
            "panic!();",
            "let x = foo().unwrap();",
            "foo().expect(\"boom\");",
            "todo!();",
            "unimplemented!();",
        ] {
            let src = file(&format!("{body}\n"));
            assert_eq!(classes(&src)[0], LineClass::Ignored, "body: {body}");
        }
    }

    #[test]
    fn unwrap_or_is_not_ignored() {
        // unwrap_or / unwrap_or_default don't panic — must NOT be ignored.
        let src = file("let x = o.unwrap_or(0);\nlet y = o.unwrap_or_default();\n");
        assert_eq!(classes(&src)[0], LineClass::Uncovered);
        assert_eq!(classes(&src)[1], LineClass::Uncovered);
    }

    #[test]
    fn ignore_region_and_line() {
        let src = file(
            "let a = 1; //cov:ignore-line\n//cov:ignore-start\nlet b = 2;\n//cov:ignore-stop\n",
        );
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Ignored);
        assert_eq!(c[2], LineClass::Ignored);
    }

    #[test]
    fn excluded_but_covered_is_pink() {
        let mut src = file("let a = 1; //cov:excl-line\n");
        // Mark line 1 as covered.
        src.lines.insert("1".to_string(), vec![]);
        assert_eq!(classes(&src)[0], LineClass::ExcludedCovered);
    }

    #[test]
    fn ignored_but_covered_stays_grey() {
        // Ignored+covered is still grey (not pink); only excluded gets the pink
        // "covered anyway" treatment.
        let mut src = file("let a = foo().unwrap();\n");
        src.lines.insert("1".to_string(), vec![]);
        assert_eq!(classes(&src)[0], LineClass::Ignored);
    }

    #[test]
    fn non_executable_lines_are_none_and_not_counted() {
        let mut src = file("let a = 1;\n// comment\n");
        // Make line 2 non-executable (a comment).
        src.executable = vec![1];
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Uncovered);
        assert_eq!(c[1], LineClass::None);
        let s = stats(&c);
        assert_eq!(s.coverable, 1);
        assert_eq!(s.covered, 0);
    }

    #[test]
    fn stats_and_pct() {
        // 4 executable: 2 covered, 1 excluded, 1 ignored.
        let mut src = SourceFile {
            content: "a\nb\nc\nd\n".to_string(),
            lines: BTreeMap::from([("1".to_string(), vec![]), ("2".to_string(), vec![])]),
            above_threshold: BTreeMap::new(),
            executable: vec![1, 2, 3, 4],
        };
        // Make 3 excluded, 4 ignored.
        src.content = "a\nb\nc //cov:excl-line\nd.unwrap()\n".to_string();
        let c = classes(&src);
        let s = stats(&c);
        assert_eq!(s.coverable, 2);
        assert_eq!(s.covered, 2);
        assert_eq!(s.excluded, 1);
        assert_eq!(s.excluded_covered, 0);
        assert_eq!(s.ignored, 1);
        assert_eq!(s.pct(), Some(100));
    }

    #[test]
    fn stats_split_excluded_and_excluded_covered() {
        // Two excluded lines: line 1 uncovered (pure excluded), line 2 covered
        // anyway (excluded-but-covered). The two counts must be tallied
        // separately so the report can surface stale markers on their own.
        let src = SourceFile {
            content: "a //cov:excl-line\nb //cov:excl-line\n".to_string(),
            lines: BTreeMap::from([("2".to_string(), vec![0])]),
            above_threshold: BTreeMap::new(),
            executable: vec![1, 2],
        };
        let c = classify(&src);
        assert_eq!(c[0], LineClass::Excluded);
        assert_eq!(c[1], LineClass::ExcludedCovered);
        let s = stats(&c);
        assert_eq!(s.excluded, 1);
        assert_eq!(s.excluded_covered, 1);
        // Neither counts as coverable — excluded lines opt out of the total.
        assert_eq!(s.coverable, 0);
    }

    // --- multi-line macro head phantom detection ---------------------------

    #[test]
    fn multiline_macro_head_phantom_is_ignored() {
        // The classic matches! artifact: the head line is executable but never
        // covered, while its argument lines (the same statement) are covered.
        let src = covered(
            file(
                "    let needs = actions.iter().any(|a| {\n\
                 matches!(\n\
                 a.as_str(),\n\
                 \"X\" | \"Y\"\n\
                 )\n\
                 });\n",
            ),
            &[1, 3, 4, 5, 6],
        );
        let c = classes(&src);
        assert_eq!(c[0], LineClass::Covered); // let … any(|a| {
        assert_eq!(c[1], LineClass::Ignored); // matches!(  ← phantom
        assert_eq!(c[2], LineClass::Covered); // a.as_str(),
        assert_eq!(c[3], LineClass::Covered); // "X" | "Y"
        assert_eq!(c[4], LineClass::Covered); // )
        assert_eq!(c[5], LineClass::Covered); // });
        // The phantom is noise, not a gap: ignored (not coverable) → still 100%.
        // Before this fix it read as Uncovered, dragging the file to 83%.
        let s = stats(&c);
        assert_eq!(s.coverable, 5);
        assert_eq!(s.covered, 5);
        assert_eq!(s.ignored, 1);
        assert_eq!(s.pct(), Some(100));
    }

    #[test]
    fn multiline_macro_in_unhit_branch_stays_uncovered() {
        // A multi-line format! in a branch no test takes: head AND body are
        // uncovered — a real gap. The head must NOT be suppressed, because its
        // following line is also uncovered.
        let src = file(
            "let s = if cond {\n\
             format!(\n\
             \"hi {}\",\n\
             name\n\
             )\n\
             } else {\n\
             String::new()\n\
             };\n",
        );
        let c = classes(&src);
        assert_eq!(c[1], LineClass::Uncovered); // format!(  ← real gap
        assert_eq!(c[2], LineClass::Uncovered); // "hi {}",
    }

    #[test]
    fn nested_macro_head_phantom_is_ignored() {
        // errors.push(format!( … )): llvm-cov marks the format! head uncovered
        // while the outer .push( and the string/arg lines are covered.
        let src = covered(
            file("errors.push(format!(\n\"err: {}\",\nmsg\n));\n"),
            &[2, 3, 4],
        );
        assert_eq!(classes(&src)[0], LineClass::Ignored); // errors.push(format!(
    }

    #[test]
    fn single_line_macro_uncovered_stays_uncovered() {
        // A single-line macro has no phantom (head and args share a line), so
        // an uncovered one is a genuine gap — must not be suppressed.
        let src = file("let s = format!(\"hi {}\", name);\n");
        assert_eq!(classes(&src)[0], LineClass::Uncovered);
    }

    #[test]
    fn covered_multiline_macro_head_is_covered() {
        // If the head is somehow covered (it usually isn't — that's the
        // artifact), it must read as covered, not ignored.
        let mut src = file("format!(\n\"x\"\n)\n");
        src.lines.insert("1".to_string(), vec![]);
        assert_eq!(classes(&src)[0], LineClass::Covered);
    }

    #[test]
    fn opens_multiline_macro_detects_heads() {
        let f = opens_multiline_macro;
        // multi-line heads (opener left open)
        assert!(f("        matches!("));
        assert!(f("        write!("));
        assert!(f("        errors.push(format!("));
        assert!(f("    foo! {")); // braced macro
        assert!(f("    bar![")); // bracketed macro
        // single-line / balanced → not a multi-line head
        assert!(!f("    let s = format!(\"hi {}\", x);"));
        assert!(!f("    matches!(a.as_str(), \"X\")"));
        // not a macro at all
        assert!(!f("    if cond {"));
        assert!(!f("    let x = foo(")); // function call, no `!`
        assert!(!f("    if a != 0 {")); // `!=` is not a macro
        // a stray `foo!(` in a trailing comment must not trigger
        assert!(!f("    let x = 1; // see format!("));
    }

    // --- single-test ("unique") coverage ---------------------------------

    #[test]
    fn single_covering_test_is_unique() {
        // A line covered by exactly one test → orange "unique" dot; covered by
        // two tests → plain green. Unique is a subset of covered, so both still
        // count toward the coverage total.
        let mut src = file("let a = 1;\nlet b = 2;\n");
        src.lines.insert("1".to_string(), vec![0]);
        src.lines.insert("2".to_string(), vec![0, 1]);
        let c = classes(&src);
        assert_eq!(c[0], LineClass::CoveredUnique);
        assert_eq!(c[1], LineClass::Covered);
        let s = stats(&c);
        assert_eq!(s.coverable, 2);
        assert_eq!(s.covered, 2);
        assert_eq!(s.unique, 1);
    }

    #[test]
    fn above_threshold_line_is_not_unique() {
        // A line covered by many tests is only known to be covered (we keep a
        // sample, not every test), and it can never be a single point of
        // failure — so it must read as plain covered, not unique.
        let mut src = file("let a = 1;\n");
        src.above_threshold.insert(
            "1".to_string(),
            crate::report::database::AboveThreshold { total: 12, sample: vec![3] },
        );
        assert_eq!(classes(&src)[0], LineClass::Covered);
    }

    #[test]
    fn unique_is_suppressed_by_excluded_and_ignored() {
        // A singly-covered line that is also excluded/ignored keeps its
        // exclusion/ignore classification — "unique" only refines plain
        // covered lines.
        let mut a = file("let a = 1; //cov:excl-line\n");
        a.lines.insert("1".to_string(), vec![0]);
        assert_eq!(classes(&a)[0], LineClass::ExcludedCovered);

        let mut b = file("let a = foo().unwrap();\n");
        b.lines.insert("1".to_string(), vec![0]);
        assert_eq!(classes(&b)[0], LineClass::Ignored);
    }

    // --- #[test] fn bodies are excluded from "unique" ----------------------

    #[test]
    fn test_fn_body_is_not_unique() {
        // A #[test] fn's body is covered only by itself — that's expected, not
        // a single point of failure, so it must read as plain covered (green)
        // and must NOT count toward the unique stat.
        let mut src = file("#[test]\nfn basic() {\n    assert_eq!(1 + 1, 2);\n}\n");
        src.lines.insert("3".to_string(), vec![0]); // the assert, covered by the sole test
        let c = classes(&src);
        assert_eq!(c[2], LineClass::Covered); // not CoveredUnique
        assert_eq!(stats(&c).unique, 0);
    }

    #[test]
    fn production_fn_uniquely_covered_is_unique() {
        // A normal (non-test) fn whose body only one test covers stays orange.
        let mut src = file("fn helper() {\n    1 + 1\n}\n");
        src.executable = vec![1, 2, 3];
        src.lines.insert("2".to_string(), vec![0]);
        assert_eq!(classes(&src)[1], LineClass::CoveredUnique);
    }

    #[test]
    fn async_test_attr_recognised() {
        // #[tokio::test] and other `…::test` attributes mark the body too.
        let mut src = file("#[tokio::test]\nasync fn it() {\n    do_thing().await;\n}\n");
        src.lines.insert("3".to_string(), vec![0]);
        assert_eq!(classes(&src)[2], LineClass::Covered);
    }

    #[test]
    fn brace_in_string_does_not_close_test_body_early() {
        // A '}' inside a format string must not end the body scan early — the
        // real closing brace is on a later line. Both body lines land inside
        // the test fn → plain covered, not unique.
        let mut src = file(
            "#[test]\nfn with_fmt() {\n    let s = format!(\"}\");\n    assert!(s.is_empty());\n}\n",
        );
        src.lines.insert("3".to_string(), vec![0]);
        src.lines.insert("4".to_string(), vec![0]);
        let c = classes(&src);
        assert_eq!(c[2], LineClass::Covered);
        assert_eq!(c[3], LineClass::Covered);
        assert_eq!(stats(&c).unique, 0);
    }
}
