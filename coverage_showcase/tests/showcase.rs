//! Integration tests for `coverage_showcase`.
//!
//! These hit exactly the code paths needed to make each category in
//! `src/lib.rs` render with its intended dot colour. Anything *not* called
//! here is the deliberate "not covered" / unreachable / panic example.

use coverage_showcase::*;

#[test]
fn covered_and_not() {
    // add: fully covered → green.
    assert_eq!(add(2, 3), 5);

    // fizzbuzz: exercise only the Fizz and plain-number paths, so the
    // FizzBuzz and Buzz arms stay red.
    assert_eq!(fizzbuzz(3), "Fizz");
    assert_eq!(fizzbuzz(1), "number");
}

#[test]
fn excluded_examples() {
    // classify: hit the A arm and the "C or below" arm; leave the excluded
    // "B" arm unreached → white.
    assert_eq!(classify(95), "A");
    assert_eq!(classify(50), "C or below");

    // traffic_light: cover R/G/Y; the unreachable `_` arm stays white.
    assert_eq!(traffic_light(b'R'), "red");
    assert_eq!(traffic_light(b'G'), "green");
    assert_eq!(traffic_light(b'Y'), "yellow");
}

#[test]
fn excluded_but_covered_is_pink() {
    // double's body is reached, yet it carries an excl-line marker → pink.
    assert_eq!(double(21), 42);
}

#[test]
fn ignored_panic_sites() {
    // These exercise the lines around the panic sites; the panic lines
    // themselves are grey (ignored) whether or not they're reached.
    assert_eq!(force_parse("42"), 42);
    assert_eq!(first_half("hello"), "he");
    assert_eq!(parse_or_die(Some(7)), 7);
    assert_eq!(variant_handler(0, 9), 9);
    assert_eq!(noisy_default("ab"), 2);

    // The panic / todo / unimplemented arms are deliberately NOT hit: they
    // would abort, and they're grey either way.
}
