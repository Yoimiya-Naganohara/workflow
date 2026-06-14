//! Token counting for the TUI status bar.
//!
//! Uses `tiktoken-rs` (cl100k_base encoding) for accurate LLM token estimation.
//! Falls back to char-based estimation if the BPE file hasn't been downloaded yet.

use tiktoken_rs::CoreBPE;

/// Get the singleton cl100k_base tokenizer.
/// This lazily downloads the BPE file on first call.
fn tokenizer() -> Option<&'static CoreBPE> {
    // cl100k_base_singleton uses lazy_static internally — the first call
    // downloads the BPE file (~1 MB) and caches it for subsequent calls.
    // We wrap in catch-unwind to handle the case where the download fails
    // (e.g. no network during first run).
    std::panic::catch_unwind(|| tiktoken_rs::cl100k_base_singleton()).ok()
}

/// Count tokens in `text` using cl100k_base encoding.
///
/// Falls back to `text.chars().count().div_ceil(4)` if the tokenizer hasn't
/// been initialised yet (e.g. first run with no network).
pub fn count_tokens(text: &str) -> u32 {
    if let Some(bpe) = tokenizer() {
        return bpe.encode_with_special_tokens(text).len() as u32;
    }
    // Fallback: ~4 chars per token
    (text.chars().count() as u32).div_ceil(4)
}

/// Check if the tokenizer has been initialised (BPE file is ready).
pub fn is_initialised() -> bool {
    tokenizer().is_some()
}

/// Ensure the tokenizer is initialised. Call once at startup to eagerly
/// download the BPE file rather than waiting until first status bar render.
pub fn init() {
    let _ = tokenizer();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_fallback() {
        let n = count_tokens("Hello, world!");
        // 13 chars / 4 = 3.25 → ceil = 4
        assert_eq!(n, 4);
    }

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_count_tokens_unicode() {
        // Chinese characters: each char is 1 char
        let n = count_tokens("你好世界");
        assert_eq!(n, 1, "4 chars / 4 = 1");
    }
}
