export type AgentId = number;

export interface AgentInfo {
	id: AgentId;
	role: string;
	current_task: string | null;
	state: string;
}

export type AgentStatus = "idle" | "thinking" | "running-tool" | "responding" | "error";

export type ConversationMessage =
	| { type: "user"; text: string }
	| { type: "text"; text: string }
	| { type: "thinking"; text: string }
	| { type: "tool"; text: string; result: string | null; is_error?: boolean }
	| { type: "error"; text: string };

export interface RuntimeSnapshot {
	agents: AgentInfo[];
	selected: AgentId | null;
	messages: ConversationMessage[];
}

export interface RoleInfo {
	id: string;
	name: string;
	definition: string;
}

export type UiEvent =
	| { type: "agent_added"; agent_id: AgentId }
	| { type: "agent_removed"; agent_id: AgentId }
	| { type: "agent_stopped"; agent_id: AgentId }
	| { type: "agent_output"; agent_id: AgentId }
	| { type: "transcript_changed"; agent_id: AgentId }
	| { type: "roles_changed" }
	| { type: "resync_required" }
	| { type: "error"; message: string }
	| { type: "mcp_connected"; server: string; tool_count: number }
	| { type: "mcp_disconnected"; server: string }
	| { type: "mcp_tool_needs_approval"; request_id: string; server: string; tool: string; arguments: Record<string, unknown> };

export interface PinnedMessage {
	id: number;
	chatItemId: number;
	text: string;
	type: "user" | "text" | "thinking" | "tool" | "error";
	result?: string | null;
	status?: "done" | "running" | "error";
	timestamp: number;
	agentId: AgentId | null;
	agentRole?: string;
}

export type DialogId = "new-agent" | "mcp-approval";

// ── Session types ────────────────────────────────────────────────

export interface SessionMeta {
	id: number;
	name: string;
	created_at: number;
	last_used_at: number;
}

export type PendingAction =
	| { type: "send"; agentId: AgentId }
	| { type: "create-agent" }
	| { type: "remove-agent"; agentId: AgentId }
	| { type: "add-role" }
	| { type: "refresh-providers" }
	| null;

export interface ProviderModel {
	id: string;
	name: string;
	supports_tools: boolean;
}

export interface ProviderEntry {
	id: string;
	name: string;
	api_url: string | null;
	models: ProviderModel[];
}

export interface ChatItem {
	id: number;
	type: "user" | "text" | "thinking" | "tool" | "error";
	text: string;
	result?: string | null;
	status?: "running" | "done" | "error";
	streaming?: boolean;
}

// ── MCP types ─────────────────────────────────────────────────

export interface LogEntry {
	ts: number;
	event: UiEvent;
}

export interface McpConnectionInfo {
	name: string;
	tool_names: string[];
}

export interface McpServerConfig {
	name: string;
	transport: McpTransport;
	dangerous_tools?: string[];
}

export type McpTransport =
	| { type: "stdio"; command: string; args: string[]; env?: Record<string, string> }
	| { type: "sse"; url: string }
	| { type: "streamable_http"; url: string };

/** Data payload for custom SvelteFlow AgentNode */
export interface AgentNodeData extends Record<string, unknown> {
	id: number;
	role: string;
	task: string | null;
	status: AgentStatus;
	roleColor: string;
	expanded?: boolean;
	chatItems?: ChatItem[];
}
