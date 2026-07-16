import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
import type {
	AgentId, AgentInfo, AgentStatus,
	ConversationMessage, RuntimeSnapshot, RoleInfo,
	UiEvent, DialogId, TabId, PendingAction, ChatItem,
	ProviderEntry,
} from "./types";

export interface LogEntry {
	ts: number;
	event: UiEvent;
}

class AppState {
	agents = $state<AgentInfo[]>([]);
	selected = $state<AgentId | null>(null);
	messages = $state<ConversationMessage[]>([]);

	dialog = $state<DialogId | null>(null);
	pendingAction = $state<PendingAction>(null);
	error = $state("");
	currentTab = $state<TabId>("chat");
	rolesExpanded = $state(false);

	roles = $state<RoleInfo[]>([]);
	configured = $state(false);

	providers = $state<ProviderEntry[]>([]);
	selectedProvider = $state<string>("");
	selectedModel = $state<string>("");
	settingsApiKey = $state("");

	input = $state("");

	eventLog = $state<LogEntry[]>([]);

	#unlisten: (() => void) | null = null;

	chatItems: ChatItem[] = $derived.by(() => {
		return this.messages.map((m, i) => {
			if (m.type === "text") {
				let html = "";
				try { html = String(marked.parse(m.text, { async: false })); } catch { /* ignore */ }
				return { id: i, type: "assistant", text: m.text, html };
			}
			if (m.type === "tool") {
				return {
					id: i, type: "tool", text: m.text,
					result: m.result, status: m.result ? "done" : "running",
				};
			}
			return { id: i, type: m.type, text: m.text };
		});
	});

	agentStatuses: Map<AgentId, AgentStatus> = $derived.by(() => {
		const map = new Map<AgentId, AgentStatus>();
		for (const a of this.agents) {
			if (a.current_task) {
				map.set(a.id, "thinking");
			} else {
				map.set(a.id, "idle");
			}
		}
		for (const m of this.messages) {
			if (m.type === "thinking") {
				map.set(this.selected ?? 0, "thinking");
			} else if (m.type === "tool" && m.result === null) {
				map.set(this.selected ?? 0, "running-tool");
			} else if (m.type === "error") {
				map.set(this.selected ?? 0, "error");
			} else if (m.type === "text") {
				map.set(this.selected ?? 0, "responding");
			}
		}
		return map;
	});

	openDialog = (id: DialogId) => { this.dialog = id; };
	closeDialog = () => { this.dialog = null; };
	setTab = (tab: TabId) => { this.currentTab = tab; };
	toggleRoles = () => { this.rolesExpanded = !this.rolesExpanded; };

	loadRoles = async () => {
		try {
			this.roles.length = 0;
			this.roles.push(...(await invoke("get_roles")) as RoleInfo[]);
		} catch (e) {
			this.error = `load roles: ${e}`;
		}
	};

	pull = async (sel?: AgentId | null) => {
		try {
			const s = await invoke("snapshot", { selected: sel ?? this.selected }) as RuntimeSnapshot;
			this.agents.length = 0;
			this.agents.push(...s.agents);
			if (s.selected !== null && s.selected !== undefined) {
				this.selected = s.selected as AgentId;
			}
			this.messages.length = 0;
			this.messages.push(...s.messages);
			this.error = "";
		} catch (e) {
			this.error = `snapshot: ${e}`;
		}
	};

	submit = async () => {
		if (!this.input.trim() || this.selected == null) return;
		const text = this.input.trim();
		this.input = "";
		this.pendingAction = { type: "send", agentId: this.selected };
		try {
			const s = await invoke("send", { target: this.selected, text }) as RuntimeSnapshot;
			this.agents.length = 0;
			this.agents.push(...s.agents);
			this.selected = s.selected as AgentId;
			this.messages.length = 0;
			this.messages.push(...s.messages);
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
			const updated = await invoke("create_agent", { roleName: role }) as AgentInfo[];
			this.agents.length = 0;
			this.agents.push(...updated);
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
			const updated = await invoke("remove_agent", { id }) as AgentInfo[];
			this.agents.length = 0;
			this.agents.push(...updated);
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

	addRole = async (name: string, def: string) => {
		if (!name.trim() || !def.trim()) return;
		this.pendingAction = { type: "add-role" };
		try {
			const updated = await invoke("add_role", { name: name.trim(), definition: def.trim() }) as RoleInfo[];
			this.roles.length = 0;
			this.roles.push(...updated);
		} catch (e) {
			this.error = `add role: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	startOrchestration = async (description: string) => {
		if (!this.selected) { this.error = "Select an agent first"; return; }
		const text = `Please orchestrate the following mission by creating sub-agents for each task:\n\n${description}`;
		this.setTab("chat");
		this.pendingAction = { type: "send", agentId: this.selected };
		try {
			const s = await invoke("send", { target: this.selected, text }) as RuntimeSnapshot;
			this.agents.length = 0;
			this.agents.push(...s.agents);
			this.selected = s.selected as AgentId;
			this.messages.length = 0;
			this.messages.push(...s.messages);
			this.error = "";
		} catch (e) {
			this.error = `orchestrate: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	selectAgent = (id: AgentId) => {
		this.selected = id;
		this.pull(id);
	};

	loadProviders = async () => {
		try {
			this.providers.length = 0;
			this.providers.push(...(await invoke("list_providers")) as ProviderEntry[]);
		} catch (e) {
			this.error = `load providers: ${e}`;
		}
	};

	refreshProviders = async () => {
		this.pendingAction = { type: "refresh-providers" };
		try {
			const fetched = await invoke("fetch_providers") as ProviderEntry[];
			this.providers.length = 0;
			this.providers.push(...fetched);
			this.error = "";
		} catch (e) {
			this.error = e instanceof Error ? e.message : `refresh providers: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	configureRuntime = async (providerId: string, apiKey: string, model: string) => {
		try {
			await invoke("configure_runtime", { providerId, apiKey, model });
			this.selectedProvider = providerId;
			this.selectedModel = model;
			this.configured = true;
			this.error = "";
			this.closeDialog();
			await this.pull(null);
		} catch (e) {
			this.error = `configure: ${e}`;
		}
	};

	init = async () => {
		await this.loadRoles();
		await this.pull(null);
		await this.loadProviders();
		this.refreshProviders();
		this.#unlisten = await listen<UiEvent>("workflow:event", (event) => {
			const entry: LogEntry = { ts: Date.now(), event: event.payload };
			this.eventLog.push(entry);
			if (this.eventLog.length > 500) this.eventLog.splice(0, this.eventLog.length - 500);
			if (event.payload.type === "error") {
				this.error = event.payload.message ?? "runtime error";
				return;
			}
			this.pull();
		});
	};

	destroy = () => {
		this.#unlisten?.();
	};
}

export const state = new AppState();
