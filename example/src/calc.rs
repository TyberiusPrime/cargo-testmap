//! Basic integer arithmetic with explicit branches.

pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

pub fn sub(a: i64, b: i64) -> i64 {
    a - b
}

pub fn mul(a: i64, b: i64) -> i64 {
    a * b
}

/// Returns `None` on division by zero. The zero-check is a branch that is only
/// covered by a test that deliberately divides by zero.
pub fn div(a: i64, b: i64) -> Option<i64> {
    if b == 0 {
        return None;
    }
    Some(a / b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_works() {
        assert_eq!(add(2, 2), 4);
        assert_eq!(add(-1, 1), 0);
    }

    #[test]
    fn sub_works() {
        assert_eq!(sub(5, 3), 2);
    }
}

