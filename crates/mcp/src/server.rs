//! MCP Server mode — expose workflow tools as an MCP server.
//!
//! This is a placeholder for future use. When enabled, it will allow external
//! MCP clients (e.g. Claude Desktop) to connect and use workflow's built-in
//! tools via the MCP protocol.
//!
//! ## Future direction
//!
//! - Create an `rmcp::ServerHandler` that wraps the workflow `ToolServerHandle`.
//! - Support stdio and Streamable HTTP transports.
//! - Advertise all tools registered in the `ToolServerHandle`.
//! - Forward tool calls to the `ToolServerHandle::call_tool()`.
