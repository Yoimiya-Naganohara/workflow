//! Structured data tools: extract_json for parsing semi-structured text.
//!
//! Agents frequently need to parse command output, log files, or config
//! formats into structured JSON.  Attempting this via `sh` + `jq` is fragile:
//! quoting, escaping, and missing tools create hard-to-debug failures.
//!
//! `extract_json` provides a sandboxed Rust-native pipeline:
//!
//! 1. Parse input text with a named-capture-group regex
//! 2. Return structured JSON with proper escaping and types
//!
//! When no pattern is supplied, it falls back to JSON validation + pretty-print
//! with enriched error messages (position + context).

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use super::builtin::ToolCallError;

// ── ExtractJson ──

#[derive(Deserialize)]
pub struct ExtractJsonArgs {
    /// Free-form input text (command output, log, config, etc.).
    pub input: String,
    /// Optional regex with named capture groups, e.g.
    /// `(?P<key>\w+)=(?P<value>\d+)` produces `[{"key": "x", "value": "42"}]`.
    ///
    /// When omitted, the tool validates input as JSON and pretty-prints it.
    pub pattern: Option<String>,
    /// When true, deduplicate result entries before returning.
    /// Only meaningful when `pattern` is supplied (regex extraction).
    pub dedup: Option<bool>,
}

pub struct ExtractJson;

impl Tool for ExtractJson {
    const NAME: &'static str = "extract_json";

    type Error = ToolCallError;
    type Args = ExtractJsonArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Extract structured data from text. ",
                "Two modes:\n",
                "1. **Regex mode**: supply a `pattern` with named capture groups ",
                "(e.g. `(?P<name>\\w+)=(?P<age>\\d+)`) — returns an array ",
                "of all matches as JSON objects.\n",
                "2. **Validate mode**: omit `pattern` — validates the input as JSON, ",
                "pretty-prints it, or returns a detailed parse error."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["input"],
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Free-form text to extract from (command output, logs, config, etc.)"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Rust regex with named capture groups, e.g. `(?P<key>\\w+)=(?P<value>\\d+)`",
                        "optional": true
                    },
                    "dedup": {
                        "type": "boolean",
                        "description": "Deduplicate result entries. Only used with `pattern` (default: false)",
                        "optional": true
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.pattern {
            Some(ref pat) => {
                if pat.trim().is_empty() {
                    return Err(ToolCallError(
                        "pattern must not be empty when provided".to_string(),
                    ));
                }
                extract_with_regex(&args.input, pat, args.dedup.unwrap_or(false))
            }
            None => validate_json(&args.input),
        }
    }
}

/// Extract structured data using a regex with named capture groups.
fn extract_with_regex(input: &str, pattern: &str, dedup: bool) -> Result<String, ToolCallError> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| ToolCallError(format!("Invalid regex '{}': {}", pattern, e)))?;

    // Collect named capture group names in order
    let group_names: Vec<String> = re
        .capture_names()
        .filter_map(|n| n.map(|s| s.to_string()))
        .collect();

    if group_names.is_empty() {
        return Err(ToolCallError(
            "Regex has no named capture groups. Use (?P<name>...) syntax, e.g. \
             `(?P<key>\\w+)=(?P<value>\\d+)`"
                .to_string(),
        ));
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cap in re.captures_iter(input) {
        let mut obj = serde_json::Map::new();
        for name in &group_names {
            let value = cap.name(name).map(|m| m.as_str()).unwrap_or("");
            obj.insert(
                name.clone(),
                // Attempt numeric parsing for ergonomic output
                try_parse_number(value),
            );
        }
        let value = serde_json::Value::Object(obj);

        if dedup {
            let canonical = format!("{:?}", value);
            if seen.contains(&canonical) {
                continue;
            }
            seen.insert(canonical);
        }

        results.push(value);
    }

    let total = results.len();
    let output = serde_json::to_string_pretty(&results)
        .map_err(|e| ToolCallError(format!("Failed to serialize results: {}", e)))?;

    Ok(format!(
        "Extracted {} record(s) using regex:\n{}\n---\nFields: [{}]",
        total,
        output,
        group_names.join(", ")
    ))
}

/// Try to parse a string as i64 → f64 → keep-as-string.
fn try_parse_number(s: &str) -> serde_json::Value {
    // Try integer first
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    // Try float (with decimal point)
    if s.contains('.') {
        if let Ok(n) = s.parse::<f64>() {
            // Only use float if it's finite
            if n.is_finite() {
                return serde_json::json!(n);
            }
        }
    }
    serde_json::Value::String(s.to_string())
}

/// Validate input as JSON and pretty-print it.
fn validate_json(input: &str) -> Result<String, ToolCallError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Ok("(empty input — nothing to parse)".to_string());
    }

    // Attempt to parse as JSON
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value)
                .map_err(|e| ToolCallError(format!("Serialization error: {}", e)))?;

            let kind = match &value {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            };

            let meta = match &value {
                serde_json::Value::Object(o) => format!("{} top-level keys", o.len()),
                serde_json::Value::Array(a) => format!("{} elements", a.len()),
                _ => "scalar value".to_string(),
            };

            Ok(format!(
                "Valid JSON ({}):\n---\n{}\n---\n{} bytes — {}",
                kind,
                pretty,
                trimmed.len(),
                meta
            ))
        }
        Err(e) => {
            // Enriched error: show position + context
            let pos = e.column();
            let line = e.line();

            // Find the problematic line and location
            let context = extract_error_context(trimmed, line, pos);

            Err(ToolCallError(format!(
                "JSON parse error at line {}, column {}: {}\n---\n{}---",
                line, pos, e, context,
            )))
        }
    }
}

/// Extract an error context snippet showing the problematic location.
fn extract_error_context(input: &str, err_line: usize, err_col: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();

    // Show 2 lines before and after the error line
    let start = err_line.saturating_sub(3).max(0);
    let end = (err_line + 2).min(lines.len());

    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let abs_line = start + i + 1;
        out.push_str(&format!("{:>4}: {}\n", abs_line, line));

        if abs_line == err_line && err_col > 0 && err_col <= line.len() {
            // Show caret under the error column
            let caret = format!("{:width$}^", "", width = err_col + 5); // +5 for line number prefix
            out.push_str(&format!("{}\n", caret));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_with_regex ──

    #[test]
    fn test_regex_basic_named_groups() {
        let input = "name=alice,role=admin\nname=bob,role=viewer";
        let result =
            extract_with_regex(input, r"name=(?P<name>\w+),role=(?P<role>\w+)", false).unwrap();
        assert!(result.contains("alice"));
        assert!(result.contains("admin"));
        assert!(result.contains("bob"));
        assert!(result.contains("viewer"));
        assert!(result.contains("2 record(s)"));
    }

    #[test]
    fn test_regex_dedup() {
        let input = "x=1 y=2\nx=1 y=2\nx=3 y=4";
        let result = extract_with_regex(input, r"(?P<x>\d+)\s+y=(?P<y>\d+)", true).unwrap();
        // 2 unique entries instead of 3 raw matches
        assert!(
            result.contains("2 record(s)"),
            "dedup should reduce to 2, got:\n{}",
            result
        );
    }

    #[test]
    fn test_regex_no_named_groups() {
        let input = "hello world";
        let result = extract_with_regex(input, r"\w+", false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no named capture groups")
        );
    }

    #[test]
    fn test_regex_invalid_pattern() {
        let result = extract_with_regex("test", r"[unclosed", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }

    #[test]
    fn test_regex_number_parsing() {
        let input = "count=42 ratio=3.14 name=hello";
        let result = extract_with_regex(input, r"(?P<key>\w+)=(?P<value>\S+)", false).unwrap();
        // "42" and "3.14" should be parsed as numbers in JSON
        assert!(
            result.contains("\"value\": 42"),
            "integer should be number, got:\n{}",
            result
        );
        assert!(
            result.contains("\"value\": 3.14"),
            "float should be number, got:\n{}",
            result
        );
        assert!(
            result.contains("\"value\": \"hello\""),
            "string should stay string, got:\n{}",
            result
        );
    }

    // ── validate_json ──

    #[test]
    fn test_validate_json_object() {
        let input = r#"{"name": "alice", "age": 30}"#;
        let result = validate_json(input).unwrap();
        assert!(result.contains("Valid JSON"));
        assert!(result.contains("object"));
        assert!(result.contains("2 top-level keys"));
    }

    #[test]
    fn test_validate_json_array() {
        let input = r#"[1, 2, 3]"#;
        let result = validate_json(input).unwrap();
        assert!(result.contains("array"));
        assert!(result.contains("3 elements"));
    }

    #[test]
    fn test_validate_json_invalid() {
        let input = r#"{"name": "alice", age: 30}"#; // missing quotes on key
        let result = validate_json(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("JSON parse error"), "got: {}", err);
        assert!(err.contains("column"), "should show column: {}", err);
        assert!(
            err.contains("caret") || err.contains("^"),
            "should show error marker: {}",
            err
        );
    }

    #[test]
    fn test_validate_json_empty_input() {
        let result = validate_json("").unwrap();
        assert!(result.contains("empty input"));
    }

    #[test]
    fn test_validate_json_whitespace_only() {
        let result = validate_json("   \n  \t  ").unwrap();
        assert!(result.contains("empty input"));
    }

    #[test]
    fn test_validate_json_pretty_print() {
        let input = r#"{"a":1,"b":[2,3]}"#;
        let result = validate_json(input).unwrap();
        assert!(result.contains("\"a\": 1"));
        assert!(result.contains("\"b\": ["));
    }

    // ── try_parse_number ──

    #[test]
    fn test_try_parse_number_integer() {
        assert_eq!(try_parse_number("42"), serde_json::json!(42));
        assert_eq!(try_parse_number("-1"), serde_json::json!(-1));
        assert_eq!(try_parse_number("0"), serde_json::json!(0));
    }

    #[test]
    fn test_try_parse_number_float() {
        assert_eq!(try_parse_number("3.14"), serde_json::json!(3.14));
        assert_eq!(try_parse_number("-0.5"), serde_json::json!(-0.5));
    }

    #[test]
    fn test_try_parse_number_string() {
        assert_eq!(
            try_parse_number("hello"),
            serde_json::Value::String("hello".to_string())
        );
        assert_eq!(
            try_parse_number(""),
            serde_json::Value::String("".to_string())
        );
    }

    // ── extract_error_context ──

    #[test]
    fn test_extract_error_context_basic() {
        let input = "line1\nline2\nline3\nline4\nline5";
        let ctx = extract_error_context(input, 3, 2);
        assert!(ctx.contains("2: line2"));
        assert!(ctx.contains("3: line3"));
        assert!(ctx.contains("4: line4"));
        assert!(ctx.contains("^"), "should show caret position: {:?}", ctx);
    }

    #[test]
    fn test_extract_error_context_near_start() {
        let input = "only one line";
        let ctx = extract_error_context(input, 1, 5);
        assert!(ctx.contains("1: only one line"));
    }

    // ── Full Tool integration ──

    #[tokio::test]
    async fn test_extract_json_tool_definition() {
        let tool = ExtractJson;
        let def = tool.definition(String::new()).await;
        assert_eq!(def.name, "extract_json");
        assert!(def.description.contains("regex") || def.description.contains("Regex"));
        assert!(def.parameters.get("required").is_some());
    }

    #[tokio::test]
    async fn test_extract_json_tool_call_validate() {
        let tool = ExtractJson;
        let result = tool
            .call(ExtractJsonArgs {
                input: r#"{"foo": "bar"}"#.to_string(),
                pattern: None,
                dedup: None,
            })
            .await
            .unwrap();
        assert!(result.contains("Valid JSON"));
        assert!(result.contains("object"));
    }

    #[tokio::test]
    async fn test_extract_json_tool_call_regex() {
        let tool = ExtractJson;
        let result = tool
            .call(ExtractJsonArgs {
                input: "a=1 b=2\nc=3 d=4".to_string(),
                pattern: Some(r"(?P<key>[a-z])=(?P<val>\d+)".to_string()),
                dedup: None,
            })
            .await
            .unwrap();
        assert!(result.contains("4 record(s)"));
        assert!(result.contains("\"key\": \"a\""));
    }

    #[tokio::test]
    async fn test_extract_json_tool_empty_pattern_rejected() {
        let tool = ExtractJson;
        let result = tool
            .call(ExtractJsonArgs {
                input: "test".to_string(),
                pattern: Some("".to_string()),
                dedup: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }
}
