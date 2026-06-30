//! A tiny tokenizer for `"a OP b"` expressions.
//!
//! Only the four basic operators are supported. Everything else parses to
//! `None`, which is the interesting "unhappy path" exercised by tests.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

/// Parse `"  12  +  30 "` style input into `(lhs, Op, rhs)`.
pub fn parse(input: &str) -> Option<(i64, Op, i64)> {
    let mut tokens = input.split_whitespace();

    let lhs: i64 = tokens.next()?.parse().ok()?;
    let op = tokens.next()?;
    let rhs: i64 = tokens.next()?.parse().ok()?;

    // Reject inputs with trailing junk.
    if tokens.next().is_some() {
        return None;
    }

    let op = match op {
        "+" => Op::Add,
        "-" => Op::Sub,
        "*" => Op::Mul,
        "/" => Op::Div,
        _ => return None,
    };
    Some((lhs, op, rhs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add() {
        assert_eq!(parse("1 + 2"), Some((1, Op::Add, 2)));
    }

    #[test]
    fn parses_div() {
        assert_eq!(parse("8 / 2"), Some((8, Op::Div, 2)));
    }

    #[test]
    fn rejects_bad_operator() {
        assert_eq!(parse("1 ^ 2"), None);
    }

    #[test]
    fn rejects_trailing_junk() {
        assert_eq!(parse("1 + 2 extra"), None);
    }
}
