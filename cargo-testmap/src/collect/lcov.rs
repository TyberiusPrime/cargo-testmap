/// A minimal LCOV parser. We only care about which (file, line) pairs were
/// executed (count > 0). See DESIGN §9.4.
///
/// Relevant records:
///   SF:/abs/path/to/file.rs
///   DA:42,1            # line 42, count 1
///   end_of_record
pub fn parse_covered(lcov: &str) -> Vec<(String, Vec<u32>)> {
    let mut records = Vec::new();
    let mut cur_file: Option<String> = None;
    let mut cur_lines: Vec<u32> = Vec::new();

    for raw in lcov.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(f) = line.strip_prefix("SF:") {
            cur_file = Some(f.to_string());
            cur_lines.clear();
        } else if let Some(rest) = line.strip_prefix("DA:") {
            // DA:<line>,<count>[,<checksum>]
            let mut parts = rest.split(',');
            let ln = parts.next().and_then(|s| s.trim().parse::<u32>().ok());
            let cnt = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
            if let (Some(ln), Some(cnt)) = (ln, cnt)
                && cnt > 0 {
                    cur_lines.push(ln);
                }
        } else if line == "end_of_record"
            && let Some(f) = cur_file.take()
                && !cur_lines.is_empty() {
                    records.push((f, std::mem::take(&mut cur_lines)));
                }
    }
    // Handle a trailing record missing end_of_record gracefully.
    if let Some(f) = cur_file.take()
        && !cur_lines.is_empty() {
            records.push((f, cur_lines));
        }
    records
}
