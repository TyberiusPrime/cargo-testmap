//! `coverage_showcase` — one file that produces *every* line category
//! `cargo-testmap` can report, so the generated report doubles as a visual
//! legend.
//!
//! Build it with:
//!
//! ```sh
//! cargo testmap run --lib --tests --theme base16-mocha.dark
//! # then open target/testmap/report/ and click into src/lib.rs
//! ```
//!
//! Each block below is labelled with the gutter-dot colour it produces.
//! (Marker names are written without the `cov:` prefix in this prose so the
//! text itself doesn't get detected as a real marker.)

// ════════════════════════════════════════════════════════════════════════
// covered (green) — reached by at least one test
// ════════════════════════════════════════════════════════════════════════

/// Plain arithmetic, fully exercised → every executable line is green.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

// ════════════════════════════════════════════════════════════════════════
// not covered (red) — a real gap, no test reaches it
// ════════════════════════════════════════════════════════════════════════

/// FizzBuzz with deliberately-missing tests: the `"FizzBuzz"` and `"Buzz"`
/// arms are never hit, so they show up red.
pub fn fizzbuzz(n: u32) -> &'static str {
    if n % 15 == 0 {
        "FizzBuzz"
    } else if n % 3 == 0 {
        "Fizz"
    } else if n % 5 == 0 {
        "Buzz"
    } else {
        "number"
    }
}

// ════════════════════════════════════════════════════════════════════════
// excluded (white) — coverage is explicitly *not* expected
// ════════════════════════════════════════════════════════════════════════

/// An `excl-start` … `excl-stop` region excludes the lines it wraps. Here
/// the `"B"` arm is excluded (and unreached) → white.
pub fn classify(score: u32) -> &'static str {
    if score >= 90 {
        "A"
    } else if score >= 80 {
        //cov:excl-start
        "B" // grade curve not finalised yet
        //cov:excl-stop
    } else {
        "C or below"
    }
}

/// Any line containing `unreachable!` is auto-excluded (white), not ignored.
pub fn traffic_light(code: u8) -> &'static str {
    match code {
        b'R' => "red",
        b'G' => "green",
        b'Y' => "yellow",
        _ => unreachable!("only R/G/Y are valid"),
    }
}

// ════════════════════════════════════════════════════════════════════════
// excluded but covered (pink) — the marker is probably stale
// ════════════════════════════════════════════════════════════════════════

/// This line IS reached by a test, yet carries an `excl-line` marker → pink,
/// signalling the exclusion is out of date.
pub fn double(x: i32) -> i32 {
    x * 2 //cov:excl-line
}

// ════════════════════════════════════════════════════════════════════════
// ignored (grey) — panic-shaped noise, not counted as coverable
// ════════════════════════════════════════════════════════════════════════

/// `.expect(` is a panic site → greyed out and not counted.
pub fn force_parse(s: &str) -> u32 {
    s.parse().expect("valid number")
}

/// `.unwrap()` is a panic site → greyed out and not counted.
pub fn first_half(s: &str) -> &str {
    let mid = s.len().checked_div(2).unwrap();
    &s[..mid]
}

/// A `panic!` site in an untaken match arm → greyed out. The function itself
/// is exercised (the `Some` arm is tested), so only the panic arm is grey.
pub fn parse_or_die(input: Option<u32>) -> u32 {
    match input {
        Some(v) => v,
        None => panic!("missing value"),
    }
}

/// `todo!` and `unimplemented!` are panic sites too. Putting them in untaken
/// match arms shows them as clean grey dots without a red function-entry line.
pub fn variant_handler(kind: u8, n: u32) -> u32 {
    match kind {
        0 => n,
        1 => todo!("handle kind 1"),
        _ => unimplemented!("kind 2+ not done yet"),
    }
}

/// Explicit ignore markers: an `ignore-line` and an `ignore-start` …
/// `ignore-stop` region.
pub fn noisy_default(k: &str) -> u32 {
    let _ = k.is_empty(); //cov:ignore-line
    //cov:ignore-start
    let scratch = k.len() as u32;
    //cov:ignore-stop
    scratch
}
