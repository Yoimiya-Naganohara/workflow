//! Dynamic tool availability toggles.
//!
//! [`DynamicTools`] implements [`rig::vector_store::VectorStoreIndexDyn`] to
//! conditionally expose the `create_role` tool. When the flag is `true`, the
//! index returns the tool definition and the LLM sees it as available. When
//! `false`, the index returns empty and the LLM has no knowledge of the tool.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rig::vector_store::{VectorSearchRequest, VectorStoreIndexDyn, VectorStoreError};
use rig::wasm_compat::WasmBoxedFuture;

/// Toggles whether the `create_role` tool is injected into LLM prompts.
///
/// The flag is shared via [`Arc<AtomicBool>`] so multiple references (the tool
/// server index, [`Orchestrate`](crate::orchestrate::Orchestrate), etc.) all
/// observe the same value.
pub struct DynamicTools {
    create_role: Arc<AtomicBool>,
}

impl DynamicTools {
    pub fn new() -> Self {
        Self { create_role: Arc::new(AtomicBool::new(false)) }
    }

    /// Create a tools instance that shares the given flag.
    ///
    /// All clones of the flag observe writes from any owner.
    pub fn with_flag(flag: Arc<AtomicBool>) -> Self {
        Self { create_role: flag }
    }

    pub fn set_create_role(&mut self, enabled: bool) {
        self.create_role.store(enabled, Ordering::Relaxed)
    }

    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.create_role)
    }
}

impl VectorStoreIndexDyn for DynamicTools {
    fn top_n<'a>(
        &'a self,
        _req: VectorSearchRequest<rig::vector_store::request::Filter<serde_json::Value>>,
    ) -> WasmBoxedFuture<'a, Result<Vec<(f64, String, serde_json::Value)>, VectorStoreError>> {
        let enabled = self.create_role.load(Ordering::Relaxed);
        Box::pin(async move {
            if !enabled {
                return Ok(Vec::new());
            }
            Ok(vec![(
                1.0,
                "create_role".to_string(),
                serde_json::json!({
                    "name": "create_role",
                    "description": "Create a new agent role with a name and definition. \
                        The definition should describe the role's responsibilities and behavior.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Unique role identifier (e.g. 'coder', 'reviewer')"
                            },
                            "definition": {
                                "type": "string",
                                "description": "Description of the role's responsibilities and behavior"
                            }
                        },
                        "required": ["name", "definition"]
                    }
                }),
            )])
        })
    }

    fn top_n_ids<'a>(
        &'a self,
        _req: VectorSearchRequest<rig::vector_store::request::Filter<serde_json::Value>>,
    ) -> WasmBoxedFuture<'a, Result<Vec<(f64, String)>, VectorStoreError>> {
        let enabled = self.create_role.load(Ordering::Relaxed);
        Box::pin(async move {
            if !enabled {
                return Ok(Vec::new());
            }
            Ok(vec![(1.0, "create_role".to_string())])
        })
    }
}
