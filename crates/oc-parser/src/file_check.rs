use crate::error::ParserError;

/// Maximum file size in bytes (1 MB).
const MAX_FILE_SIZE: u64 = 1_048_576;

/// Number of leading bytes to inspect for binary detection.
const BINARY_CHECK_SIZE: usize = 8192;

/// Check that a file is within size limits.
/// Returns `Err(ParserError::FileTooLarge)` if the file exceeds 1 MB.
pub fn check_file_size(path: &str, size: u64) -> Result<(), ParserError> {
    if size > MAX_FILE_SIZE {
        return Err(ParserError::FileTooLarge {
            path: path.to_string(),
            size,
            max: MAX_FILE_SIZE,
        });
    }
    Ok(())
}

/// Returns `true` if the buffer appears to contain binary (non-text) data.
/// Detection: presence of null bytes in the first 8 KB.
pub fn is_binary(content: &[u8]) -> bool {
    let check_len = content.len().min(BINARY_CHECK_SIZE);
    content[..check_len].contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_within_limit() {
        assert!(check_file_size("test.py", 500_000).is_ok());
    }

    #[test]
    fn file_at_limit() {
        assert!(check_file_size("test.py", MAX_FILE_SIZE).is_ok());
    }

    #[test]
    fn file_over_limit() {
        let err = check_file_size("test.py", MAX_FILE_SIZE + 1).unwrap_err();
        match err {
            ParserError::FileTooLarge { size, max, .. } => {
                assert_eq!(size, MAX_FILE_SIZE + 1);
                assert_eq!(max, MAX_FILE_SIZE);
            }
            _ => panic!("expected FileTooLarge"),
        }
    }

    #[test]
    fn text_content_not_binary() {
        assert!(!is_binary(b"def hello():\n    pass\n"));
    }

    #[test]
    fn binary_content_detected() {
        let mut data = vec![0u8; 100];
        data[50] = 0; // explicit null
        assert!(is_binary(&data));
    }

    #[test]
    fn null_in_text_is_binary() {
        assert!(is_binary(b"hello\x00world"));
    }

    #[test]
    fn empty_content_not_binary() {
        assert!(!is_binary(b""));
    }

    #[test]
    fn null_after_8kb_not_detected() {
        let mut data = vec![b'a'; 10_000];
        data[9000] = 0;
        // Only first 8KB checked — null at byte 9000 should not be detected
        assert!(!is_binary(&data));
    }
}
