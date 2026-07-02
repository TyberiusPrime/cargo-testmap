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
//! A line is **ignored** when it is panic-shaped noise we don't want muddying
//! the numbers:
//!   - inside a `//cov:ignore-start` … `//cov:ignore-stop` region,
//!   - tagged with `//cov:ignore-line`, or
//!   - a panic site: `panic!`, `.unwrap()`, `.expect(`, `todo!`,
//!     `unimplemented!`.
//!
//! Everything else executable is either **covered** (some test reached it) or
//! **uncovered** (a real coverage gap). Non-executable lines (blanks,
//! comments, bare braces) carry no dot and aren't counted.
//!
//! Dot colors (see style.css / ThemeColors):
//!   - covered            → green
//!   - uncovered          → red
//!   - excluded           → white        (pink if covered anyway)
//!   - ignored            → grey

use crate::report::database::SourceFile;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineClass {
    /// Not executable (blank/comment/etc.) — no dot, not counted.
    None,
    /// Executable and reached by ≥1 test.
    Covered,
    /// Executable, not excluded/ignored, not reached by any test — a gap.
    Uncovered,
    /// Explicitly excluded (`excl-*` / `unreachable!`) and *not* covered.
    Excluded,
    /// Excluded but covered anyway — usually a stale exclusion marker.
    ExcludedCovered,
    /// Ignored panic-shaped line (`ignore-*` / `.unwrap()` / `panic!` / …).
    Ignored,
}

impl LineClass {
    /// The `<tr>` CSS class used to color this line's gutter dot.
    pub fn css_class(self) -> Option<&'static str> {
        match self {
            LineClass::None => None,
            LineClass::Covered => Some("cov-covered"),
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
            LineClass::Uncovered => "uncovered — no test reached this line",
            LineClass::Excluded => "excluded — coverage not expected (excl marker / unreachable!)",
            LineClass::ExcludedCovered => {
                "excluded but covered anyway — the exclusion marker may be stale"
            }
            LineClass::Ignored => "ignored — panic site (unwrap/expect/panic!/…) or ignore marker",
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
    /// Executable lines marked excluded.
    pub excluded: u32,
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
    let executable: std::collections::BTreeSet<u32> = src.executable.iter().copied().collect();
    let covered: std::collections::BTreeSet<u32> = src
        .lines
        .keys()
        .chain(src.above_threshold.keys())
        .filter_map(|k| k.parse::<u32>().ok())
        .collect();

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
        let ign = ignored[i];
        let cov = covered.contains(&lineno);
        let class = if excl && cov {
            LineClass::ExcludedCovered
        } else if excl {
            LineClass::Excluded
        } else if ign {
            LineClass::Ignored
        } else if cov {
            LineClass::Covered
        } else {
            LineClass::Uncovered
        };
        out.push(class);
    }
    out
}

/// Tally a file's classification vector into [`LineStats`].
pub fn stats(classes: &[LineClass]) -> LineStats {
    let mut s = LineStats::default();
    for &c in classes {
        match c {
            LineClass::Covered => {
                s.coverable += 1;
                s.covered += 1;
            }
            LineClass::Uncovered => s.coverable += 1,
            LineClass::Excluded | LineClass::ExcludedCovered => s.excluded += 1,
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
/// for executable lines. Marker lines themselves are never flagged by their
/// own range toggle (they're comments, hence non-executable, but this keeps
/// the semantics clean and predictable).
fn compute_excl_ignored(content: &str) -> (Vec<bool>, Vec<bool>) {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut excluded = vec![false; n];
    let mut ignored = vec![false; n];
    let mut excl_on = false;
    let mut ign_on = false;

    for (i, raw) in lines.iter().enumerate() {
        let has_excl_start = raw.contains("cov:excl-start");
        let has_excl_stop = raw.contains("cov:excl-stop");
        let has_excl_line = raw.contains("cov:excl-line");
        let has_ign_start = raw.contains("cov:ignore-start");
        let has_ign_stop = raw.contains("cov:ignore-stop");
        let has_ign_line = raw.contains("cov:ignore-line");

        // A line carrying a range toggle is itself just a marker: it is not
        // classified by the region it opens/closes.
        let is_range_marker =
            has_excl_start || has_excl_stop || has_ign_start || has_ign_stop;

        if !is_range_marker {
            if excl_on {
                excluded[i] = true;
            }
            if ign_on {
                ignored[i] = true;
            }
        }
        // Single-line markers apply to the line they sit on.
        if has_excl_line {
            excluded[i] = true;
        }
        if has_ign_line {
            ignored[i] = true;
        }
        // Automatic classifications (only meaningful on executable lines).
        if raw.contains("unreachable!") {
            excluded[i] = true;
        }
        if is_panic_line(raw) {
            ignored[i] = true;
        }

        // Advance region state for subsequent lines.
        if has_excl_start {
            excl_on = true;
        }
        if has_excl_stop {
            excl_on = false;
        }
        if has_ign_start {
            ign_on = true;
        }
        if has_ign_stop {
            ign_on = false;
        }
    }
    (excluded, ignored)
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
    fn excl_start_stop_region_excludes_inside() {
        let src = file(
            "let a = 1;\n//cov:excl-start\nlet b = 2;\nlet c = 3;\n//cov:excl-stop\nlet d = 4;\n",
        );
        let c = classes(&src);
        // line 1: uncovered; line 2: marker (excluded flag, but not executable
        // in reality — here it's executable so it shows Excluded); lines 3-4:
        // excluded; line 5: marker Excluded; line 6: uncovered.
        assert_eq!(c[0], LineClass::Uncovered);
        assert_eq!(c[2], LineClass::Excluded);
        assert_eq!(c[3], LineClass::Excluded);
        assert_eq!(c[5], LineClass::Uncovered);
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
        assert_eq!(s.ignored, 1);
        assert_eq!(s.pct(), Some(100));
    }
}
