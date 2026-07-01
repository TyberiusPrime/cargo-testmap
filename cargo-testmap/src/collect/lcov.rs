/// A minimal LCOV parser. We only care about which (file, line) pairs are
/// *executable* (have a `DA` record at all) and which were *executed*
/// (count > 0). See DESIGN §9.4.
///
/// Relevant records:
///   SF:/abs/path/to/file.rs
///   DA:42,1            # line 42, count 1
///   end_of_record
///
/// Every `DA` record names an executable (instrumented) line; its count tells
/// us whether it was run. We report both so the database can carry the full
/// executable set (needed to compute coverage gaps and classify lines as
/// covered / uncovered / excluded / ignored in the report).
pub struct FileCoverage {
    /// Every line that has a `DA` record (instrumented/executable), sorted.
    pub executable: Vec<u32>,
    /// The subset of executable lines with `count > 0` (executed by this run).
    pub covered: Vec<u32>,
}

pub fn parse(lcov: &str) -> Vec<(String, FileCoverage)> {
    let mut records = Vec::new();
    let mut cur_file: Option<String> = None;
    let mut cur_executable: Vec<u32> = Vec::new();
    let mut cur_covered: Vec<u32> = Vec::new();

    let flush = |records: &mut Vec<(String, FileCoverage)>,
                 file: Option<String>,
                 executable: &mut Vec<u32>,
                 covered: &mut Vec<u32>| {
        if let Some(f) = file
            && !executable.is_empty()
        {
            executable.sort_unstable();
            executable.dedup();
            covered.sort_unstable();
            covered.dedup();
            records.push((
                f,
                FileCoverage {
                    executable: std::mem::take(executable),
                    covered: std::mem::take(covered),
                },
            ));
        }
    };

    for raw in lcov.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(f) = line.strip_prefix("SF:") {
            flush(&mut records, cur_file.take(), &mut cur_executable, &mut cur_covered);
            cur_file = Some(f.to_string());
        } else if let Some(rest) = line.strip_prefix("DA:") {
            // DA:<line>,<count>[,<checksum>]
            let mut parts = rest.split(',');
            let ln = parts.next().and_then(|s| s.trim().parse::<u32>().ok());
            let cnt = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
            if let (Some(ln), Some(cnt)) = (ln, cnt) {
                cur_executable.push(ln);
                if cnt > 0 {
                    cur_covered.push(ln);
                }
            }
        } else if line == "end_of_record" {
            flush(&mut records, cur_file.take(), &mut cur_executable, &mut cur_covered);
        }
    }
    // Handle a trailing record missing end_of_record gracefully.
    flush(&mut records, cur_file.take(), &mut cur_executable, &mut cur_covered);
    records
}
