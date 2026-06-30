//! `calc` — a tiny arithmetic library used as a `cargo-testmap` test fixture.
//!
//! It exposes a couple of modules and a few public functions that are exercised
//! by both inline unit tests and an integration test in `tests/integration.rs`.

pub mod calc;
pub mod parser;

pub use calc::{add, div, mul, sub};

/// A convenience wrapper that evaluates `"a OP b"` strings via [`parser`] and
/// [`calc`]. Exists so that integration tests and unit tests can converge on a
/// shared code path (i.e. some lines end up covered by *several* tests).
pub fn eval(expr: &str) -> Option<i64> {
    let (a, op, b) = parser::parse(expr)?;
    let r = match op {
        parser::Op::Add => add(a, b),
        parser::Op::Sub => sub(a, b),
        parser::Op::Mul => mul(a, b),
        parser::Op::Div => div(a, b)?,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_add() {
        assert_eq!(eval("2 + 3"), Some(5));
    }

    #[test]
    fn eval_sub() {
        assert_eq!(eval("10 - 4"), Some(6));
    }

    #[test]
    fn eval_rejects_garbage() {
        // No valid operator → parse returns None → eval returns None.
        assert_eq!(eval("not an expression"), None);
    }

    // NOTE: `div`, `mul`, and the division-by-zero branch are intentionally
    // left to the integration test so that the reverse map shows them covered
    // by a single, different test.
}
