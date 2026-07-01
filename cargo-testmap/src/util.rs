use anyhow::Context;
use std::path::{Component, Path, PathBuf};

/// Strip `root` from `path`, returning a relative path. Returns None if `path`
/// is not under `root`.
pub fn relativize(path: &Path, root: &Path) -> Option<PathBuf> {
    let path = path.strip_prefix(".").unwrap_or(path);
    let root = root.strip_prefix(".").unwrap_or(root);
    let path = dunce(path);
    let root = dunce(root);
    path.strip_prefix(&root).ok().map(|p| p.to_path_buf())
}

/// Canonicalize a path for comparison purposes. We avoid full `canonicalize`
/// (which requires the file to exist and resolves symlinks aggressively),
/// and instead normalize away `.` components.
fn dunce(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Format the current time as an RFC3339 UTC string (e.g. 2026-06-30T22:00:00Z).
pub fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let sod = (secs % 86_400) as u32;
    let h = sod / 3600;
    let m = (sod % 3600) / 60;
    let s = sod % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days since the Unix epoch (1970-01-01) to a (year, month, day) triple.
/// Howard Hinnant's algorithm — valid for any proleptic Gregorian date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A tiny FNV-1a hash for deterministic staging filenames.
pub fn fnv1a(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Content fingerprint of a file (blake3 hex). Used to detect when a built
/// test binary changes — any code or test edit that makes cargo rebuild it
/// yields a different fingerprint, which invalidates that test's staged cache
/// entry so coverage is re-collected instead of served stale.
pub fn fingerprint_file(path: &Path) -> anyhow::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("fingerprinting {}", path.display()))?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}
