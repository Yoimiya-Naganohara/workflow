import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initHighlighter, setTheme } from "./markdown/highlighter";
import { LANGUAGES, THEMES, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME, getActiveTheme, setActiveTheme } from "./markdown/languages";

import type {
	AgentId,
	AgentInfo,
	AgentStatus,
	ChatItem,
	RuntimeSnapshot,
	SessionMeta,
} from "./types";
export type { LogEntry } from "./types";

import { AgentStore } from "./stores/agent-store.svelte.js";
import { ChatStore } from "./stores/chat-store.svelte.js";
import { ConfigStore } from "./stores/config-store.svelte.js";
import { SessionStore } from "./stores/session-store.svelte.js";

class AppState {
	agent = new AgentStore();
	chat = new ChatStore();
	config = new ConfigStore();
	session = new SessionStore();

	// ── Private fields ─────────────────────────────────────────
	#unlisten: (() => void) | null = null;
	#observer: MutationObserver | null = null;
	#pullPromise: Promise<void> | null = null;
	#pullQueued = false;

	// ── Backward-compatible getters / setters ──────────────────

	// Agent properties
	get agents() { return this.agent.agents; }
	set agents(v) { this.agent.agents = v; }
	get selected() { return this.agent.selected; }
	set selected(v) { this.agent.selected = v; }
	get dialog() { return this.agent.dialog; }
	set dialog(v) { this.agent.dialog = v; }
	get pendingAction() { return this.agent.pendingAction; }
	set pendingAction(v) { this.agent.pendingAction = v; }
	get error() { return this.agent.error; }
	set error(v) { this.agent.error = v; }
	get rolesExpanded() { return this.agent.rolesExpanded; }
	set rolesExpanded(v) { this.agent.rolesExpanded = v; }
	get roles() { return this.agent.roles; }
	set roles(v) { this.agent.roles = v; }

	// Chat properties
	get messages() { return this.chat.messages; }
	set messages(v) { this.chat.messages = v; }
	get input() { return this.chat.input; }
	set input(v) { this.chat.input = v; }
	get running() { return this.chat.running; }
	set running(v) { this.chat.running = v; }
	get pinnedMessages() { return this.chat.pinnedMessages; }
	set pinnedMessages(v) { this.chat.pinnedMessages = v; }
	get eventLog() { return this.chat.eventLog; }
	set eventLog(v) { this.chat.eventLog = v; }

	// Config properties
	get providers() { return this.config.providers; }
	set providers(v) { this.config.providers = v; }
	get selectedProvider() { return this.config.selectedProvider; }
	set selectedProvider(v) { this.config.selectedProvider = v; }
	get selectedModel() { return this.config.selectedModel; }
	set selectedModel(v) { this.config.selectedModel = v; }
	get settingsApiKey() { return this.config.settingsApiKey; }
	set settingsApiKey(v) { this.config.settingsApiKey = v; }
	get configured() { return this.config.configured; }
	set configured(v) { this.config.configured = v; }
	get projectPath() { return this.config.projectPath; }
	set projectPath(v) { this.config.projectPath = v; }
	get projectName() { return this.config.projectName; }
	set projectName(v) { this.config.projectName = v; }
	get projectExpanded() { return this.config.projectExpanded; }
	set projectExpanded(v) { this.config.projectExpanded = v; }
	get mcpServers() { return this.config.mcpServers; }
	set mcpServers(v) { this.config.mcpServers = v; }
	get pendingMcpApproval() { return this.config.pendingMcpApproval; }
	set pendingMcpApproval(v) { this.config.pendingMcpApproval = v; }
	get mcpConfigs() { return this.config.mcpConfigs; }
	set mcpConfigs(v) { this.config.mcpConfigs = v; }
	get mcpConnections() { return this.config.mcpConnections; }
	set mcpConnections(v) { this.config.mcpConnections = v; }
	get mcpExpanded() { return this.config.mcpExpanded; }
	set mcpExpanded(v) { this.config.mcpExpanded = v; }

	// ── Session delegation ──────────────────────────────────────
	get sessions() { return this.session.sessions; }
	set sessions(v) { this.session.sessions = v; }
	get activeSessionId() { return this.session.activeId; }
	set activeSessionId(v) { this.session.activeId = v; }

	loadSessions = () => this.session.loadSessions();

	createSession = async (name: string) => {
		await this.session.createSession(name);
		await this.pull(null);
		this.loadRoles();
	};

	switchSession = async (id: number) => {
		await this.session.switchSession(id);
		await this.pull(null);
		this.loadRoles();
	};

	deleteSession = async (id: number) => {
		await this.session.deleteSession(id);
		if (this.activeSessionId != null) {
			await this.pull(null);
			this.loadRoles();
		}
	};

	renameSession = (id: number, name: string) => this.session.renameSession(id, name);
	saveSessions = () => this.session.saveSessions();

	// ── Derived properties ─────────────────────────────────────

	get chatItems() { return this.chat.chatItems; }

	agentStatuses: Map<AgentId, AgentStatus> = $derived.by(() => {
		const map = new Map<AgentId, AgentStatus>();
		for (const a of this.agents) {
			map.set(a.id, a.current_task ? "thinking" : "idle");
		}
		if (this.selected == null || !this.running) return map;
		for (let i = this.messages.length - 1; i >= 0; i--) {
			const m = this.messages[i];
			if (m.type === "user") continue;
			const st: AgentStatus | null =
				m.type === "thinking"
					? "thinking"
					: m.type === "tool" && m.result === null
						? "running-tool"
						: m.type === "error"
							? "error"
							: m.type === "text"
								? "responding"
								: null;
			if (st) {
				map.set(this.selected, st);
				break;
			}
		}
		return map;
	});

	// ── Cross-store methods ────────────────────────────────────

	stop = async () => {
		if (this.selected == null) return;
		try {
			await invoke("stop_agent", { target: this.selected });
			this.running = false;
		} catch (e) {
			this.error = `stop: ${e}`;
		}
	};

	selectAgent = (id: AgentId) => {
		this.selected = id;
		this.pull(id);
	};

	pull = async (sel?: AgentId | null) => {
		if (this.#pullPromise) {
			this.#pullQueued = true;
			return;
		}
		const exec = async () => {
			while (true) {
				this.#pullQueued = false;
				try {
					const s = (await invoke("snapshot", {
						selected: sel ?? this.selected,
					})) as RuntimeSnapshot;
					this.agents = s.agents;
					if (s.selected !== null && s.selected !== undefined) {
						this.selected = s.selected as AgentId;
					}
					this.messages = s.messages;
					this.error = "";
					if (
						this.running &&
						this.selected != null &&
						this.pendingAction?.type !== "send" &&
						!this.agents.find((a) => a.id === this.selected)?.current_task
					) {
						this.running = false;
					}
				} catch (e) {
					this.error = `snapshot: ${e}`;
				}
				if (!this.#pullQueued) break;
			}
		};
		this.#pullPromise = exec();
		try {
			await this.#pullPromise;
		} finally {
			this.#pullPromise = null;
		}
	};

	submit = async () => {
		if (!this.input.trim() || this.selected == null) return;
		const text = this.input.trim();
		this.input = "";
		this.running = true;
		this.pendingAction = { type: "send", agentId: this.selected };
		try {
			const s = (await invoke("send", {
				target: this.selected,
				text,
			})) as RuntimeSnapshot;
			this.agents = s.agents;
			this.selected = s.selected as AgentId;
			this.messages = s.messages;
			this.error = "";
		} catch (e) {
			this.error = `send: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	createAgent = async (role: string) => {
		this.pendingAction = { type: "create-agent" };
		try {
			const updated = (await invoke("create_agent", {
				roleName: role,
			})) as AgentInfo[];
			this.agents = updated;
			const last = updated[updated.length - 1];
			if (last) {
				await this.selectAgent(last.id);
			}
			this.dialog = null;
		} catch (e) {
			this.error = `create agent: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	removeAgent = async (id: AgentId) => {
		this.pendingAction = { type: "remove-agent", agentId: id };
		try {
			const updated = (await invoke("remove_agent", {
				id,
			})) as AgentInfo[];
			this.agents = updated;
			if (this.selected === id) {
				this.selected = this.agents[0]?.id ?? null;
				await this.pull(this.selected);
			}
		} catch (e) {
			this.error = `remove agent: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	// ── Dialog / error delegation ──────────────────────────────
	dismissError = () => this.agent.dismissError();
	setError = (msg: string) => this.agent.setError(msg);
	openDialog = (id: import("./types").DialogId) => this.agent.openDialog(id);
	closeDialog = () => this.agent.closeDialog();
	toggleRoles = () => this.agent.toggleRoles();

	// ── Role delegation ────────────────────────────────────────
	addRole = (name: string, def: string) => this.agent.addRole(name, def);
	loadRoles = () => this.agent.loadRoles();

	// ── Config delegation ──────────────────────────────────────
	toggleProject = () => this.config.toggleProject();
	toggleMcp = () => this.config.toggleMcp();

	loadProviders = async () => {
		try {
			await this.config.loadProviders();
		} catch (e) {
			this.error = `load providers: ${e}`;
		}
	};

	refreshProviders = async () => {
		this.pendingAction = { type: "refresh-providers" };
		try {
			await this.config.refreshProviders();
			this.error = "";
		} catch (e) {
			this.error =
				e instanceof Error ? e.message : `refresh providers: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	saveUserConfig = () => this.config.saveUserConfig();
	loadUserConfig = () => this.config.loadUserConfig();
	refreshProject = () => this.config.refreshProject();
	loadMcpConnections = () => this.config.loadMcpConnections();
	loadMcpConfigs = () => this.config.loadMcpConfigs();

	addMcpServer = async (config: import("./types").McpServerConfig) => {
		try {
			await this.config.addMcpServer(config);
		} catch (e) {
			this.error = `add mcp server: ${e}`;
		}
	};

	removeMcpServer = async (name: string) => {
		try {
			await this.config.removeMcpServer(name);
		} catch (e) {
			this.error = `remove mcp server: ${e}`;
		}
	};

	// ── Pin delegation ─────────────────────────────────────────
	togglePinMessage = (item: ChatItem) =>
		this.chat.togglePinMessage(item, this.agents, this.selected);
	unpinMessage = (pinId: number) => this.chat.unpinMessage(pinId);
	pinMessage = (item: ChatItem) =>
		this.chat.pinMessage(item, this.agents, this.selected);

	// ── MCP approval ───────────────────────────────────────────
	approveMcpTool = async () => {
		const req = this.pendingMcpApproval;
		if (!req) return;
		try {
			await invoke("approve_mcp_tool", { requestId: req.request_id, approved: true });
		} catch (e) {
			this.error = `mcp approve: ${e}`;
		} finally {
			this.pendingMcpApproval = null;
			this.closeDialog();
		}
	};

	denyMcpTool = async () => {
		const req = this.pendingMcpApproval;
		if (!req) return;
		try {
			await invoke("approve_mcp_tool", { requestId: req.request_id, approved: false });
		} catch (e) {
			this.error = `mcp deny: ${e}`;
		} finally {
			this.pendingMcpApproval = null;
			this.closeDialog();
		}
	};

	// ── Project configuration (cross-store) ────────────────────
	reconfigureProject = async (newPath: string) => {
		try {
			await invoke("configure_runtime", {
				providerId: this.selectedProvider,
				apiKey: this.settingsApiKey,
				model: this.selectedModel,
				projectPath: newPath,
			});
			this.configured = true;
			this.error = "";
			this.roles = (await invoke("load_roles")) as import("./types").RoleInfo[];
			await this.refreshProject();
			await this.pull(null);
		} catch (e) {
			this.error = `project: ${e}`;
			throw e;
		}
	};

	clearProject = async () => {
		try {
			await invoke("configure_runtime", {
				providerId: this.selectedProvider,
				apiKey: this.settingsApiKey,
				model: this.selectedModel,
				projectPath: null,
			});
			this.configured = true;
			this.projectName = "";
			this.projectPath = "";
			this.error = "";
			await this.pull(null);
		} catch (e) {
			this.error = `project: ${e}`;
			throw e;
		}
	};

	configureRuntime = async (
		providerId: string,
		apiKey: string,
		model: string,
		projectPath?: string,
	) => {
		try {
			await invoke("configure_runtime", { providerId, apiKey, model, projectPath: projectPath || null });
			this.selectedProvider = providerId;
			this.selectedModel = model;
			this.settingsApiKey = apiKey;
			this.configured = true;
			this.error = "";
			this.closeDialog();
			this.roles = (await invoke("load_roles")) as import("./types").RoleInfo[];
			await this.refreshProject();
			await this.pull(null);
			await this.saveUserConfig();
		} catch (e) {
			this.error = `configure: ${e}`;
			throw e;
		}
	};

	// ── Lifecycle ──────────────────────────────────────────────
	init = () => {
		try {
			const isDark = document.documentElement.classList.contains("dark");
			initHighlighter(LANGUAGES, THEMES, isDark ? DEFAULT_DARK_THEME : DEFAULT_LIGHT_THEME);
		} catch (e) {
			console.error("shiki init:", e);
		}

		this.loadUserConfig().then(async () => {
			if (
				this.selectedProvider &&
				this.selectedModel &&
				this.settingsApiKey &&
				!this.configured
			) {
				this.configureRuntime(
					this.selectedProvider,
					this.settingsApiKey,
					this.selectedModel,
					this.projectPath || undefined,
				);
			}
		});
		this.loadRoles();
		this.pull(null);
		this.loadProviders();
		this.loadMcpConfigs();
		this.loadMcpConnections();
		this.loadSessions();
		this.chat.loadPinnedMessages();

		const updateTheme = () => {
			const isDark = document.documentElement.classList.contains("dark");
			const theme = isDark ? DEFAULT_DARK_THEME : DEFAULT_LIGHT_THEME;
			setActiveTheme(theme);
			try {
				setTheme(theme);
			} catch {
				// highlighter may not be ready yet
			}
		};
		updateTheme();
		this.#observer = new MutationObserver(updateTheme);
		this.#observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class"],
		});

		listen<import("./types").UiEvent>("workflow:event", (event) => {
			try {
				const entry: import("./types").LogEntry = { ts: Date.now(), event: event.payload };
				this.eventLog.push(entry);
				if (this.eventLog.length > 500)
					this.eventLog.splice(0, this.eventLog.length - 500);
				if (event.payload.type === "error") {
					this.error = event.payload.message ?? "runtime error";
					return;
				}
				if (event.payload.type === "roles_changed") {
					this.loadRoles();
					return;
				}
				if (event.payload.type === "agent_stopped") {
					this.running = false;
					this.pull();
					return;
				}
				if (event.payload.type === "mcp_connected") {
					const { server, tool_count } = event.payload;
					this.config.handleMcpConnected(server, tool_count);
				}
				if (event.payload.type === "mcp_disconnected") {
					const server = (event.payload as { type: "mcp_disconnected"; server: string }).server;
					this.config.handleMcpDisconnected(server);
				}
				if (event.payload.type === "mcp_tool_needs_approval") {
					this.pendingMcpApproval = event.payload;
					this.dialog = "mcp-approval";
				}
				this.pull();
			} catch (e) {
				console.error("event handler:", e);
			}
		})
			.then((unlisten) => {
				this.#unlisten = unlisten;
			})
			.catch((e) => {
				console.error("failed to listen for events:", e);
			});
	};

	destroy = () => {
		this.#unlisten?.();
		this.#observer?.disconnect();
		this.#observer = null;
		if (this.agent.errorTimer) {
			clearTimeout(this.agent.errorTimer);
			this.agent.errorTimer = null;
		}
		this.chat.eventLog = [];
		this.messages = [];
		this.agents = [];
	};
}

export const state = new AppState();
