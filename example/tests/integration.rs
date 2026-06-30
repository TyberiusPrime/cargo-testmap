//! Integration tests that drive the public `calc` API end-to-end.
//!
//! These overlap with the inline unit tests on purpose: lines in `calc` and
//! `parser` end up covered by *both* a unit test and an integration test, so
//! the reverse map shows several tests per line for those paths.

use calc::{add, div, eval, mul, sub};
use calc::parser::{self, Op};

#[test]
fn full_arithmetic_roundtrip() {
    assert_eq!(add(1, 2), 3);
    assert_eq!(sub(3, 1), 2);
    assert_eq!(mul(4, 5), 20);
    assert_eq!(div(10, 2), Some(5));
}

#[test]
fn eval_all_operators() {
    assert_eq!(eval("6 + 1"), Some(7));
    assert_eq!(eval("6 - 1"), Some(5));
    assert_eq!(eval("6 * 2"), Some(12));
    assert_eq!(eval("6 / 2"), Some(3));
}

#[test]
fn div_by_zero_is_none() {
    // Exercises the explicit zero-check branch in `calc::div`.
    assert_eq!(div(5, 0), None);
    assert_eq!(eval("5 / 0"), None);
}

#[test]
fn parser_roundtrips_every_op() {
    assert_eq!(parser::parse("1 + 2"), Some((1, Op::Add, 2)));
    assert_eq!(parser::parse("1 - 2"), Some((1, Op::Sub, 2)));
    assert_eq!(parser::parse("1 * 2"), Some((1, Op::Mul, 2)));
    assert_eq!(parser::parse("1 / 2"), Some((1, Op::Div, 2)));
}
