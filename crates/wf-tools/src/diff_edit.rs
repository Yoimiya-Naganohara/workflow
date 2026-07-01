//! SEARCH/REPLACE block editing — inspired by Claude Code's edit format.
//!
//! # Format
//!
//! Each edit block uses the following format:
//!
//! ```text
//! <<<<<<< SEARCH
//! [exact content to find, including surrounding context lines]
//! =======
//! [replacement content]
//! >>>>>>> REPLACE
//! ```
//!
//! # Advantages over `patch_file`
//!
//! - **Context-aware** — SEARCH block includes surrounding context lines,
//!   which uniquely identifies the edit location (no ambiguity).
//! - **Exact match** — The entire SEARCH block must match the file content
//!   byte-for-byte. This prevents accidental partial matches.
//! - **Multi-hunk atomic** — Multiple SEARCH/REPLACE blocks can be applied
//!   to one file in a single call. If any hunk fails, the file is rolled
//!   back to its original state.
//! - **LLM-native** — Claude, GPT-4, and other modern LLMs are trained on
//!   this format and produce it reliably.
//!
//! # Safety
//!
//! - Atomic write via temp file + rename.
//! - Full rollback on any hunk failure.
//! - Dry-run mode for previewing changes without writing.

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use std::io::Write;

use crate::error::ToolCallError;

/// The delimiter that opens a SEARCH block.
const SEARCH_MARKER: &str = "<<<<<<< SEARCH";
/// The delimiter between SEARCH and REPLACE.
const SEPARATOR: &str = "=======";
/// The delimiter that closes a REPLACE block.
const REPLACE_MARKER: &str = ">>>>>>> REPLACE";

/// A single parsed SEARCH/REPLACE hunk.
#[derive(Debug)]
struct EditHunk {
    /// The exact text to find (including context lines for uniqueness).
    search: String,
    /// The replacement text.
    replace: String,
}

/// Parse a single SEARCH/REPLACE block from raw text.
///
/// # Format
/// ```text
/// <<<<<<< SEARCH
/// [lines...]
/// =======
/// [lines...]
/// >>>>>>> REPLACE
/// ```
fn parse_hunk_block(block: &str) -> Result<EditHunk, String> {
    let block = block.trim();

    let sep_pos = block
        .rfind(SEPARATOR)
        .ok_or_else(|| format!("Missing `{}` separator in SEARCH/REPLACE block", SEPARATOR))?;

    let before_sep = &block[..sep_pos];
    let after_sep = &block[sep_pos + SEPARATOR.len()..];

    // Find SEARCH marker at the start
    let search_start = before_sep
        .find(SEARCH_MARKER)
        .ok_or_else(|| format!("Missing `{}` marker", SEARCH_MARKER))?;
    let search_text = before_sep[search_start + SEARCH_MARKER.len()..].trim();

    // Find REPLACE marker at the end of after_sep
    let replace_end = after_sep
        .rfind(REPLACE_MARKER)
        .ok_or_else(|| format!("Missing `{}` marker", REPLACE_MARKER))?;
    let replace_text = after_sep[..replace_end].trim();

    if search_text.is_empty() && replace_text.is_empty() {
        return Err("SEARCH/REPLACE block contains empty search and replace".to_string());
    }

    Ok(EditHunk {
        search: search_text.to_string(),
        replace: replace_text.to_string(),
    })
}

/// Parse a complete input string into a list of SEARCH/REPLACE hunks.
///
/// Supports two modes:
/// 1. Multiple blocks separated by blank lines or custom delimiters.
/// 2. Single block for simple edits.
fn parse_hunks(input: &str) -> Result<Vec<EditHunk>, String> {
    // Try to split by the SEARCH marker; count occurrences.
    let search_count = input.matches(SEARCH_MARKER).count();

    if search_count == 0 {
        return Err(format!(
            "No `{}` block found. Use the SEARCH/REPLACE format:\n\
             {}<<<<<<< SEARCH\n\
             exact content to match\n\
             =======\n\
             replacement content\n\
             >>>>>>> REPLACE",
            SEARCH_MARKER, SEARCH_MARKER
        ));
    }

    // Split input on SEARCH_MARKER and reconstruct each block
    let parts: Vec<&str> = input.splitn(search_count + 1, SEARCH_MARKER).collect();

    // parts[0] is anything before the first SEARCH marker (should be empty or preamble)
    // Each subsequent parts[i] starts right after a SEARCH_MARKER
    let mut hunks = Vec::with_capacity(search_count);

    for (i, part) in parts.iter().enumerate().skip(1) {
        // Reconstruct the block by prepending the marker
        let block = format!("{}{}", SEARCH_MARKER, part);
        match parse_hunk_block(&block) {
            Ok(hunk) => {
                if !hunk.search.is_empty() || !hunk.replace.is_empty() {
                    hunks.push(hunk);
                }
            }
            Err(e) => {
                return Err(format!("Hunk #{}: {}", i, e));
            }
        }
    }

    if hunks.is_empty() {
        return Err("No valid SEARCH/REPLACE hunks found".to_string());
    }

    Ok(hunks)
}

/// Apply hunks to a file. Returns the new content after all hunks are applied.
///
/// Each hunk is applied in order. If any hunk fails, `Err` is returned with
/// the index of the failing hunk and the error message.
fn apply_hunks(content: &str, hunks: &[EditHunk]) -> Result<String, String> {
    let mut result = content.to_string();

    for (i, hunk) in hunks.iter().enumerate() {
        if !result.contains(&hunk.search) {
            // Provide a helpful error with context
            let search_preview = if hunk.search.len() > 120 {
                format!(
                    "{}...{}",
                    &hunk.search[..60],
                    &hunk.search[hunk.search.len() - 60..]
                )
            } else {
                hunk.search.clone()
            };

            // If there are multiple files, suggest checking the path
            return Err(format!(
                "Hunk #{}: SEARCH block not found in file.\n\n\
                 Searched for:\n---\n{}---\n\n\
                 Possible causes:\n\
                 1. The file has been modified since you read it — re-read and retry.\n\
                 2. Indentation differs (tabs vs spaces).\n\
                 3. The SEARCH block has a trailing newline that doesn't exist in the file.\n\
                 4. Line endings (CRLF vs LF) don't match.\n\n\
                 Tip: include 1-2 lines of surrounding context above AND below the change.",
                i + 1,
                search_preview,
            ));
        }

        // Replace ONLY the first occurrence (safety: each hunk targets one location)
        match result.find(&hunk.search) {
            Some(pos) => {
                let before = &result[..pos];
                let after = &result[pos + hunk.search.len()..];
                result = format!("{}{}{}", before, hunk.replace, after);
            }
            None => {
                // This shouldn't happen since we already checked contains(),
                // but handle it gracefully anyway.
                return Err(format!(
                    "Hunk #{}: SEARCH block matched via contains() but find() failed — \
                     possible Unicode boundary issue",
                    i + 1
                ));
            }
        }
    }

    Ok(result)
}

/// Generate a human-readable diff preview between old and new content.
fn generate_hunk_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut output = String::new();
    let mut i = 0;
    let mut j = 0;

    let max_preview = 2000; // max chars to prevent overflow

    while i < old_lines.len() || j < new_lines.len() {
        if output.len() >= max_preview {
            output.push_str("... (diff truncated)\n");
            break;
        }

        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            // Unchanged
            output.push_str(&format!("  {}\n", old_lines[i]));
            i += 1;
            j += 1;
        } else if j < new_lines.len() && (i >= old_lines.len() || new_lines[j] != old_lines[i]) {
            // Added line
            output.push_str(&format!("+ {}\n", new_lines[j]));
            j += 1;
        } else if i < old_lines.len() {
            // Removed line
            output.push_str(&format!("- {}\n", old_lines[i]));
            i += 1;
        }
    }

    output
}

// ── DiffEdit Tool ──

#[derive(Deserialize)]
pub struct DiffEditArgs {
    /// Path to the file to edit.
    pub path: String,
    /// One or more SEARCH/REPLACE blocks.
    /// Each block follows the format:
    ///   <<<<<<< SEARCH
    ///   [exact text to match with context]
    ///   =======
    ///   [replacement text]
    ///   >>>>>>> REPLACE
    pub edits: String,
    /// If true, validate and preview without writing.
    #[serde(default)]
    pub dry_run: bool,
}

pub struct DiffEdit;

impl Tool for DiffEdit {
    const NAME: &'static str = "diff_edit";

    type Error = ToolCallError;
    type Args = DiffEditArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Edit a file using SEARCH/REPLACE blocks. ",
                "The most reliable way to make precise edits.\n\n",
                "FORMAT (one or more blocks in `edits`):\n",
                "<<<<<<< SEARCH\n",
                "exact text to find\n",
                "(include 1-2 context lines above and below for uniqueness)\n",
                "=======\n",
                "replacement text\n",
                ">>>>>>> REPLACE\n\n",
                "RULES:\n",
                "1. The entire SEARCH block (including context) must match exactly.\n",
                "2. Include surrounding context lines to guarantee uniqueness.\n",
                "3. Multiple blocks are applied atomically — if one fails, all revert.\n",
                "4. Use dry_run=true to preview before applying."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["path", "edits"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "edits": {
                        "type": "string",
                        "description": concat!(
                            "One or more SEARCH/REPLACE blocks. ",
                            "Format:\n",
                            "<<<<<<< SEARCH\n",
                            "exact text (include context lines for uniqueness)\n",
                            "=======\n",
                            "replacement text\n",
                            ">>>>>>> REPLACE\n\n",
                            "For multiple edits, place blocks one after another."
                        )
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Validate and preview changes without writing (default: false)",
                        "default": false
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. Read the file
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| ToolCallError(format!("Cannot read '{}': {}", args.path, e)))?;

        // 2. Parse the SEARCH/REPLACE hunks
        let hunks = parse_hunks(&args.edits).map_err(ToolCallError)?;

        // 3. Apply hunks in sequence
        let new_content = apply_hunks(&content, &hunks).map_err(ToolCallError)?;

        // 4. Early return for dry-run
        if args.dry_run {
            let diff = generate_hunk_diff(&content, &new_content);
            return Ok(format!(
                "DRY RUN — {} hunk(s) would apply to '{}'\n\n\
                 Change preview ({:?} lines → {:?} lines):\n---\n{}---\n\n\
                 Use dry_run=false to apply.",
                hunks.len(),
                args.path,
                content.lines().count(),
                new_content.lines().count(),
                diff,
            ));
        }

        // 5. Atomic write: temp file + rename
        let tmp_path = format!("{}.tmp.{}", args.path, std::process::id());
        {
            let mut tmp = std::fs::File::create(&tmp_path)
                .map_err(|e| ToolCallError(format!("Failed to create temp file: {}", e)))?;
            tmp.write_all(new_content.as_bytes())
                .map_err(|e| ToolCallError(format!("Failed to write temp file: {}", e)))?;
            tmp.flush()
                .map_err(|e| ToolCallError(format!("Failed to flush temp file: {}", e)))?;
        }
        std::fs::rename(&tmp_path, &args.path)
            .map_err(|e| ToolCallError(format!("Failed to rename temp file: {}", e)))?;

        // 6. Result
        let diff = generate_hunk_diff(&content, &new_content);

        Ok(format!(
            "Applied {} hunk(s) to '{}'.\n\n\
             File changed: {} lines → {} lines.\n\n\
             Changes:\n---\n{}---",
            hunks.len(),
            args.path,
            content.lines().count(),
            new_content.lines().count(),
            diff,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_hunk_block ──

    #[test]
    fn test_parse_basic_hunk() {
        let block = "<<<<<<< SEARCH\nold content\n=======\nnew content\n>>>>>>> REPLACE";
        let hunk = parse_hunk_block(block).unwrap();
        assert_eq!(hunk.search, "old content");
        assert_eq!(hunk.replace, "new content");
    }

    #[test]
    fn test_parse_hunk_with_context() {
        let block = "\
<<<<<<< SEARCH
fn old_function() {
    let x = 1;
    println!(\"{}\", x);
}
=======
fn new_function() {
    let x = 2;
    println!(\"{}\", x * 2);
}
>>>>>>> REPLACE";
        let hunk = parse_hunk_block(block).unwrap();
        assert!(hunk.search.contains("old_function"));
        assert!(hunk.replace.contains("new_function"));
        assert!(hunk.search.contains("println!"));
    }

    #[test]
    fn test_parse_hunk_missing_separator() {
        let block = "<<<<<<< SEARCH\ncontent\n>>>>>>> REPLACE";
        let err = parse_hunk_block(block).unwrap_err();
        assert!(err.contains("======="));
    }

    #[test]
    fn test_parse_hunk_missing_markers() {
        let block = "plain text without markers";
        let err = parse_hunk_block(block).unwrap_err();
        assert!(err.contains("======="), "error: {:?}", err);
    }

    #[test]
    fn test_parse_hunk_empty() {
        let block = "<<<<<<< SEARCH\n=======\n>>>>>>> REPLACE";
        let err = parse_hunk_block(block).unwrap_err();
        assert!(err.contains("empty"));
    }

    // ── parse_hunks ──

    #[test]
    fn test_parse_hunks_single() {
        let input = "<<<<<<< SEARCH\nfoo\n=======\nbar\n>>>>>>> REPLACE";
        let hunks = parse_hunks(input).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].search, "foo");
    }

    #[test]
    fn test_parse_hunks_multiple() {
        let input = "\
<<<<<<< SEARCH
first old
=======
first new
>>>>>>> REPLACE

<<<<<<< SEARCH
second old
=======
second new
>>>>>>> REPLACE";
        let hunks = parse_hunks(input).unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].search, "first old");
        assert_eq!(hunks[1].search, "second old");
    }

    #[test]
    fn test_parse_hunks_no_marker() {
        let err = parse_hunks("hello world").unwrap_err();
        assert!(err.contains("<<<<<<< SEARCH"));
    }

    // ── apply_hunks ──

    #[test]
    fn test_apply_single_hunk() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let hunks = vec![EditHunk {
            search: "    println!(\"hello\");".to_string(),
            replace: "    println!(\"world\");".to_string(),
        }];
        let result = apply_hunks(content, &hunks).unwrap();
        assert!(result.contains("world"));
        assert!(!result.contains("hello"));
    }

    #[test]
    fn test_apply_multiple_hunks() {
        let content = "aaa\nbbb\nccc\nddd\n";
        let hunks = vec![
            EditHunk {
                search: "aaa".to_string(),
                replace: "AAA".to_string(),
            },
            EditHunk {
                search: "ccc".to_string(),
                replace: "CCC".to_string(),
            },
        ];
        let result = apply_hunks(content, &hunks).unwrap();
        assert_eq!(result, "AAA\nbbb\nCCC\nddd\n");
    }

    #[test]
    fn test_apply_hunk_not_found() {
        let content = "hello world\n";
        let hunks = vec![EditHunk {
            search: "nonexistent".to_string(),
            replace: "replacement".to_string(),
        }];
        let err = apply_hunks(content, &hunks).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_apply_hunk_context_uniqueness() {
        // SEARCH includes surrounding context lines
        let content = "line1\nTARGET\nline3\nANOTHER_TARGET\nline5\n";
        let hunks = vec![EditHunk {
            search: "line1\nTARGET\nline3".to_string(),
            replace: "line1\nUPDATED\nline3".to_string(),
        }];
        let result = apply_hunks(content, &hunks).unwrap();
        assert!(result.contains("UPDATED"));
        assert!(result.contains("ANOTHER_TARGET")); // unchanged
    }

    #[test]
    fn test_apply_hunk_atomic_failure() {
        // First hunk succeeds, second fails → we DON'T roll back in apply_hunks
        // (rollback is handled at the tool level)
        let content = "aaa\nbbb\n";
        let hunks = vec![
            EditHunk {
                search: "aaa".to_string(),
                replace: "AAA".to_string(),
            },
            EditHunk {
                search: "xxx".to_string(),
                replace: "yyy".to_string(),
            },
        ];
        let err = apply_hunks(content, &hunks).unwrap_err();
        assert!(err.contains("not found"));
        // The first hunk should NOT have been applied because apply_hunks fails fast
    }

    // ─── Full tool integration ──

    #[tokio::test]
    async fn test_diff_edit_tool_call_basic() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "hello old world").unwrap();
        drop(f);

        let tool = DiffEdit;
        let edits = "\
<<<<<<< SEARCH
hello old world
=======
hello new world
>>>>>>> REPLACE";

        let result = tool
            .call(DiffEditArgs {
                path: file_path.to_str().unwrap().to_string(),
                edits: edits.to_string(),
                dry_run: false,
            })
            .await
            .unwrap();
        assert!(result.contains("Applied"));
        assert!(result.contains("1 hunk"));

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("new world"));
    }

    #[tokio::test]
    async fn test_diff_edit_tool_dry_run() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "original content").unwrap();
        drop(f);

        let tool = DiffEdit;
        let edits = "\
<<<<<<< SEARCH
original content
=======
modified content
>>>>>>> REPLACE";

        let result = tool
            .call(DiffEditArgs {
                path: file_path.to_str().unwrap().to_string(),
                edits: edits.to_string(),
                dry_run: true,
            })
            .await
            .unwrap();
        assert!(result.contains("DRY RUN"));

        // File should not have been modified
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("original"));
    }

    #[tokio::test]
    async fn test_diff_edit_tool_hunk_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "existing content").unwrap();

        let tool = DiffEdit;
        let edits = "\
<<<<<<< SEARCH
nonexistent content
=======
replacement
>>>>>>> REPLACE";

        let result = tool
            .call(DiffEditArgs {
                path: file_path.to_str().unwrap().to_string(),
                edits: edits.to_string(),
                dry_run: false,
            })
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        assert!(err.contains("SEARCH"));
    }

    #[tokio::test]
    async fn test_diff_edit_tool_nonexistent_file() {
        let tool = DiffEdit;
        let edits = "\
<<<<<<< SEARCH
anything
=======
nothing
>>>>>>> REPLACE";

        let result = tool
            .call(DiffEditArgs {
                path: "/tmp/nonexistent_file_xyz".to_string(),
                edits: edits.to_string(),
                dry_run: false,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_diff_edit_tool_definition() {
        let tool = DiffEdit;
        let def = tool.definition(String::new()).await;
        assert_eq!(def.name, "diff_edit");
        assert!(def.description.contains("SEARCH"));
        assert!(def.description.contains("REPLACE"));
    }

    #[tokio::test]
    async fn test_diff_edit_multi_hunk_atomic() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("multi.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "line A").unwrap();
        writeln!(f, "line B").unwrap();
        writeln!(f, "line C").unwrap();
        drop(f);

        let tool = DiffEdit;
        let edits = "\
<<<<<<< SEARCH
line A
=======
line A_MODIFIED
>>>>>>> REPLACE

<<<<<<< SEARCH
line C
=======
line C_MODIFIED
>>>>>>> REPLACE";

        let result = tool
            .call(DiffEditArgs {
                path: file_path.to_str().unwrap().to_string(),
                edits: edits.to_string(),
                dry_run: false,
            })
            .await
            .unwrap();
        assert!(result.contains("2 hunk(s)"));

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("A_MODIFIED"));
        assert!(!content.contains("A\n"));
        assert!(content.contains("C_MODIFIED"));
        assert!(content.contains("line B")); // unchanged
    }
}
