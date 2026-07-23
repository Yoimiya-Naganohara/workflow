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
	| { type: "tool"; text: string; result: string | null }
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
	| { type: "mcp_disconnected"; server: string };

export type DialogId = "new-agent" | "settings" | "roles";

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
	type: "user" | "assistant" | "thinking" | "tool" | "error";
	text: string;
	result?: string | null;
	status?: "running" | "done";
	streaming?: boolean;
}
