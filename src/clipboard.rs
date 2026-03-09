//! System clipboard integration via OSC 52 terminal escape sequence.

use std::io::Write;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Copies `text` to the system clipboard via the OSC 52 escape sequence.
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let encoded = BASE64.encode(text.as_bytes());
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{encoded}\x07")?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encoding_works() {
        let encoded = BASE64.encode(b"hello");
        assert_eq!(encoded, "aGVsbG8=");
    }

    #[test]
    fn empty_string_does_not_panic() {
        let encoded = BASE64.encode(b"");
        assert_eq!(encoded, "");
    }
}
