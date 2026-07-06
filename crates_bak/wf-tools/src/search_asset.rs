//! Semantic retrieval tool — searches indexed assets via embedding similarity.
//!
//! `SearchAsset` queries vector embeddings built from large compilation
//! outputs or logs using SIMD cosine similarity. Requires a sandboxed
//! agent context with an attached embedding engine.
//!
//! This tool is conditionally registered — it is only added to the tool
//! server when a sandbox is available (see `builtin::register_sandboxed_tools`).

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::error::ToolCallError;

/// Arguments for searching an indexed asset.
#[derive(Deserialize)]
pub struct SearchAssetArgs {
    pub asset_id: String,
    pub query: String,
    pub top_k: Option<usize>,
}

/// Semantic search tool — retrieves chunks of an indexed asset via embedding similarity.
///
/// Only functions within a sandboxed agent context that has an embedding
/// model attached. Returns the top-K semantically relevant chunks (each ~20 lines).
pub struct SearchAsset {
    pub sandbox: Option<std::sync::Arc<wf_agent::sandbox::SandboxHandle>>,
}

impl Tool for SearchAsset {
    const NAME: &'static str = "search_asset";

    type Error = ToolCallError;
    type Args = SearchAssetArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Semantic search within a previously indexed asset. ",
                "Targeted at large compilation outputs or logs. ",
                "Avoid reading the full asset via read_file. ",
                "Returns top-K semantically relevant chunks (each ~20 lines)."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "asset_id": {
                        "type": "string",
                        "description": "Asset ID returned by Shell/ReadFile after SIMD indexing"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query — describe what you're looking for (e.g. 'unresolved import error', 'panic at main')"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Number of chunks to return (default: 3, max: 10)",
                        "minimum": 1,
                        "maximum": 10,
                        "optional": true
                    }
                },
                "required": ["asset_id", "query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let sandbox = self.sandbox.as_ref().ok_or_else(|| {
            ToolCallError("SearchAsset requires a sandboxed agent context".into())
        })?;

        let model = {
            let guard = sandbox.embedder.read().expect("embedder mutex poisoned");
            guard
                .as_ref()
                .ok_or_else(|| {
                    ToolCallError(
                        "No embedding model attached — asset was stored but not indexed".into(),
                    )
                })?
                .clone()
        };

        let query_emb = model
            .embed(&args.query)
            .await
            .map_err(|e| ToolCallError(format!("Embedding failed: {}", e)))?;

        let results = {
            let guard = sandbox
                .asset_indices
                .read()
                .expect("asset_indices mutex poisoned");
            let asset = guard.get(&args.asset_id).ok_or_else(|| {
                ToolCallError(format!(
                    "Asset '{}' not found or not indexed",
                    args.asset_id
                ))
            })?;
            asset.search(&query_emb, args.top_k.unwrap_or(3))
        };

        if results.is_empty() {
            return Ok(format!(
                "[search_asset] No relevant chunks in '{}' for: \"{}\"",
                args.asset_id, args.query
            ));
        }

        let mut out = format!(
            "=== Semantic search in '{}' — \"{}\" ===\n",
            args.asset_id, args.query
        );
        for (line_start, content) in &results {
            out.push_str(&format!("\n--- Line {} ---\n{}", line_start, content));
        }
        out.push_str(&format!("\n[returned {} chunk(s)]", results.len()));
        Ok(out)
    }
}
