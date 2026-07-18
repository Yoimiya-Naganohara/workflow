use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rig::vector_store::VectorStoreIndexDyn;

/// Toggle that controls whether `create_role` is injected into LLM prompts.
///
/// This is **not** a tool — it is a [`VectorStoreIndexDyn`] implementation
/// plugged into the shared [`ToolServer`](rig::tool::server::ToolServer) via
/// [`dynamic_tools`](rig::tool::server::ToolServer::dynamic_tools). When the
/// flag is `true` the tool server includes the `create_role` tool definition
/// in the prompt; when `false` the model never sees it.
pub struct DynamicTools {
    create_role: Arc<AtomicBool>,
}

impl DynamicTools {
    pub fn new() -> Self {
        Self {
            create_role: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_flag(flag: Arc<AtomicBool>) -> Self {
        Self { create_role: flag }
    }

    pub fn set_create_role(&mut self, v: bool) {
        self.create_role.store(v, Ordering::Relaxed)
    }

    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.create_role)
    }
}

impl VectorStoreIndexDyn for DynamicTools {
    fn top_n<'a>(
        &'a self,
        _req: rig::vector_store::VectorSearchRequest<
            rig::vector_store::request::Filter<serde_json::Value>,
        >,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, rig::vector_store::TopNResults> {
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
        _req: rig::vector_store::VectorSearchRequest<
            rig::vector_store::request::Filter<serde_json::Value>,
        >,
    ) -> rig::wasm_compat::WasmBoxedFuture<
        'a,
        Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError>,
    > {
        let enabled = self.create_role.load(Ordering::Relaxed);
        Box::pin(async move {
            if !enabled {
                return Ok(Vec::new());
            }
            Ok(vec![(1.0, "create_role".to_string())])
        })
    }
}
