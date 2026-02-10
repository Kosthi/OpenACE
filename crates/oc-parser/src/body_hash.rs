use xxhash_rust::xxh3::xxh3_128;

/// Compute body_hash: XXH3-128 lower 64 bits of the given byte slice.
pub fn compute_body_hash(body: &[u8]) -> u64 {
    xxh3_128(body) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let body = b"def foo():\n    return 42\n";
        assert_eq!(compute_body_hash(body), compute_body_hash(body));
    }

    #[test]
    fn different_content_different_hash() {
        let h1 = compute_body_hash(b"def foo(): pass");
        let h2 = compute_body_hash(b"def bar(): pass");
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_body() {
        let _ = compute_body_hash(b"");
    }
}
