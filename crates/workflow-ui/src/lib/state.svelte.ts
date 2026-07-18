import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
import type {
	AgentId, AgentInfo, AgentStatus,
	ConversationMessage, RuntimeSnapshot, RoleInfo,
	UiEvent, DialogId, PendingAction, ChatItem,
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

	#htmlCache = new Map<string, string>();

	chatItems: ChatItem[] = $derived.by(() => {
		return this.messages.map((m, i) => {
			if (m.type === "text") {
				let html = this.#htmlCache.get(m.text);
				if (html === undefined) {
					try { html = String(marked.parse(m.text, { async: false })); } catch { /* ignore */ }
					this.#htmlCache.set(m.text, html ?? "");
					if (this.#htmlCache.size > 200) {
						const key = this.#htmlCache.keys().next().value;
						if (key !== undefined) this.#htmlCache.delete(key);
					}
				}
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
			map.set(a.id, a.current_task ? "thinking" : "idle");
		}
		if (this.selected == null) return map;
		for (let i = this.messages.length - 1; i >= 0; i--) {
			const m = this.messages[i];
			if (m.type === "user") continue;
			const st: AgentStatus | null = m.type === "thinking" ? "thinking"
				: m.type === "tool" && m.result === null ? "running-tool"
				: m.type === "error" ? "error"
				: m.type === "text" ? "responding"
				: null;
			if (st) { map.set(this.selected, st); break; }
		}
		return map;
	});

	openDialog = (id: DialogId) => { this.dialog = id; };
	closeDialog = () => { this.dialog = null; };
	toggleRoles = () => { this.rolesExpanded = !this.rolesExpanded; };

	loadRoles = async () => {
		try {
			this.roles = await invoke("get_roles") as RoleInfo[];
		} catch (e) {
			this.error = `load roles: ${e}`;
		}
	};

	pull = async (sel?: AgentId | null) => {
		try {
			const s = await invoke("snapshot", { selected: sel ?? this.selected }) as RuntimeSnapshot;
			this.agents = s.agents;
			if (s.selected !== null && s.selected !== undefined) {
				this.selected = s.selected as AgentId;
			}
			this.messages = s.messages;
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
			const updated = await invoke("create_agent", { roleName: role }) as AgentInfo[];
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
			const updated = await invoke("remove_agent", { id }) as AgentInfo[];
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

	addRole = async (name: string, def: string) => {
		if (!name.trim() || !def.trim()) return;
		this.pendingAction = { type: "add-role" };
		try {
			this.roles = await invoke("add_role", { name: name.trim(), definition: def.trim() }) as RoleInfo[];
		} catch (e) {
			this.error = `add role: ${e}`;
		} finally {
			this.pendingAction = null;
		}
	};

	selectAgent = (id: AgentId) => {
		this.selected = id;
		this.pull(id);
	};

	saveUserConfig = async () => {
		try {
			await invoke("save_config", {
				config: {
					selected_provider: this.selectedProvider,
					selected_model: this.selectedModel,
					api_key: this.settingsApiKey,
				},
			});
		} catch (e) {
			console.error("save config:", e);
		}
	};

	loadUserConfig = async () => {
		try {
			const cfg = await Promise.race([
				invoke("load_config") as Promise<{ selected_provider: string; selected_model: string; api_key: string } | null>,
				new Promise<null>((_, reject) => setTimeout(() => reject(new Error("timeout")), 3000)),
			]);
			if (cfg) {
				this.selectedProvider = cfg.selected_provider;
				this.selectedModel = cfg.selected_model;
				this.settingsApiKey = cfg.api_key;
			}
		} catch (e) {
			console.error("load config:", e);
		}
	};

	loadProviders = async () => {
		try {
			this.providers = await invoke("list_providers") as ProviderEntry[];
		} catch (e) {
			this.error = `load providers: ${e}`;
		}
	};

	refreshProviders = async () => {
		this.pendingAction = { type: "refresh-providers" };
		try {
			this.providers = await invoke("fetch_providers") as ProviderEntry[];
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
			this.settingsApiKey = apiKey;
			this.configured = true;
			this.error = "";
			this.closeDialog();
			await this.pull(null);
			await this.saveUserConfig();
		} catch (e) {
			this.error = `configure: ${e}`;
		}
	};

	init = () => {
		this.loadUserConfig().then(() => {
			if (this.selectedProvider && this.selectedModel && this.settingsApiKey && !this.configured) {
				this.configureRuntime(this.selectedProvider, this.settingsApiKey, this.selectedModel);
			}
		});
		this.loadRoles();
		this.pull(null);
		this.loadProviders();
		this.refreshProviders();
		listen<UiEvent>("workflow:event", (event) => {
			const entry: LogEntry = { ts: Date.now(), event: event.payload };
			this.eventLog.push(entry);
			if (this.eventLog.length > 500) this.eventLog.splice(0, this.eventLog.length - 500);
			if (event.payload.type === "error") {
				this.error = event.payload.message ?? "runtime error";
				return;
			}
			this.pull();
		}).then((unlisten) => {
			this.#unlisten = unlisten;
		});
	};

	destroy = () => {
		this.#unlisten?.();
	};
}

export const state = new AppState();
