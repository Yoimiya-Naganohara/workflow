//! Tokenization — CLI input → ParsedCommand.

/// Tokenized but not yet resolved command input.
pub struct ParsedCommand {
    pub tokens: Vec<String>,
}

/// Split a CLI input string into tokens, respecting quotes.
/// `"/role show \"foo bar\""` → `["role", "show", "foo bar"]`
pub fn parse(input: &str) -> ParsedCommand {
    let trimmed = input.trim().strip_prefix('/').unwrap_or(input.trim());
    let tokens = parse_tokens(trimmed);
    ParsedCommand { tokens }
}

fn parse_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in input.chars() {
        match ch {
            '"' | '\'' => in_quote = !in_quote,
            ' ' | '\t' if !in_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
