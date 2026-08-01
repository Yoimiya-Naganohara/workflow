import { invoke } from "@tauri-apps/api/core";
import type {
	McpConnectionInfo,
	McpServerConfig,
	ProviderEntry,
} from "../types";

export class ConfigStore {
	providers = $state<ProviderEntry[]>([]);
	selectedProvider = $state<string>("");
	selectedModel = $state<string>("");
	settingsApiKey = $state("");
	configured = $state(false);

	projectPath = $state("");
	projectName = $state("");
	projectExpanded = $state(true);

	mcpServers = $state<{ name: string; tool_count: number }[]>([]);
	pendingMcpApproval = $state<{
		request_id: string;
		server: string;
		tool: string;
		arguments: Record<string, unknown>;
	} | null>(null);
	mcpConfigs = $state<McpServerConfig[]>([]);
	mcpConnections = $state<McpConnectionInfo[]>([]);
	mcpExpanded = $state(true);

	toggleProject = () => {
		this.projectExpanded = !this.projectExpanded;
	};

	toggleMcp = () => {
		this.mcpExpanded = !this.mcpExpanded;
	};

	// These internal methods swallow errors (no user-facing feedback needed)
	loadMcpConnections = async () => {
		try {
			this.mcpConnections =
				(await invoke("list_mcp_connections")) as McpConnectionInfo[];
		} catch (e) {
			console.error("load mcp connections:", e);
		}
	};

	loadMcpConfigs = async () => {
		try {
			this.mcpConfigs = (await invoke("list_mcp_configs")) as McpServerConfig[];
		} catch (e) {
			console.error("load mcp configs:", e);
		}
	};

	// These methods let errors propagate so root can set agent.error / agent.pendingAction
	addMcpServer = async (config: McpServerConfig) => {
		await invoke("add_mcp_server", { config });
		await this.loadMcpConfigs();
		await this.loadMcpConnections();
	};

	removeMcpServer = async (name: string) => {
		await invoke("remove_mcp_server", { name });
		await this.loadMcpConfigs();
		await this.loadMcpConnections();
	};

	handleMcpConnected = (server: string, tool_count: number) => {
		const idx = this.mcpServers.findIndex((s) => s.name === server);
		if (idx >= 0) {
			this.mcpServers[idx] = { name: server, tool_count };
		} else {
			this.mcpServers.push({ name: server, tool_count });
		}
		this.loadMcpConnections();
	};

	handleMcpDisconnected = (server: string) => {
		this.mcpServers = this.mcpServers.filter((s) => s.name !== server);
		this.loadMcpConnections();
	};

	// Provider methods — let errors propagate for root wrapping
	loadProviders = async () => {
		this.providers = (await invoke("list_providers")) as ProviderEntry[];
	};

	refreshProviders = async () => {
		this.providers = (await invoke("fetch_providers")) as ProviderEntry[];
	};

	saveUserConfig = async () => {
		try {
			await invoke("save_config", {
				config: {
					selected_provider: this.selectedProvider,
					selected_model: this.selectedModel,
					api_key: this.settingsApiKey,
					project_path: this.projectPath,
				},
			});
		} catch (e) {
			console.error("save config:", e);
		}
	};

	loadUserConfig = async () => {
		try {
			const cfg = (await invoke("load_config")) as {
				selected_provider: string;
				selected_model: string;
				api_key: string;
				project_path?: string;
			} | null;
			if (cfg) {
				this.selectedProvider = cfg.selected_provider;
				this.selectedModel = cfg.selected_model;
				this.settingsApiKey = cfg.api_key;
				if (cfg.project_path) {
					this.projectPath = cfg.project_path;
				}
			}
		} catch (e) {
			console.error("load config:", e);
		}
	};

	refreshProject = async () => {
		try {
			const info = await invoke<{ name: string; path: string } | null>("get_project");
			if (info) {
				this.projectName = info.name;
				this.projectPath = info.path;
			}
		} catch {
			// runtime not configured yet
		}
	};
}
