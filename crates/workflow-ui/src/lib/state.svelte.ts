import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initHighlighter, setTheme } from "./markdown/highlighter";
import { LANGUAGES, THEMES, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME, getActiveTheme, setActiveTheme } from "./markdown/languages";

import type {
    AgentId,
    AgentInfo,
    AgentStatus,
    ConversationMessage,
    RuntimeSnapshot,
    RoleInfo,
    UiEvent,
    DialogId,
    PendingAction,
    ChatItem,
    PinnedMessage,
    ProviderEntry,
    McpConnectionInfo,
    McpServerConfig,
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
    errorTimer: ReturnType<typeof setTimeout> | null = null;
    rolesExpanded = $state(false);

    roles = $state<RoleInfo[]>([]);
    configured = $state(false);

    providers = $state<ProviderEntry[]>([]);
    selectedProvider = $state<string>("");
    selectedModel = $state<string>("");
    settingsApiKey = $state("");

    // ── Project support ─────────────────────────────────────────
    projectPath = $state("");
    projectName = $state("");
    projectExpanded = $state(true);

    input = $state("");
    running = $state(false);

    // ── Pinned Messages ────────────────────────────────────────
    pinnedMessages = $state<PinnedMessage[]>([]);

    eventLog = $state<LogEntry[]>([]);

    // ── MCP server status ──────────────────────────────────────
    mcpServers = $state<{ name: string; tool_count: number }[]>([]);
    pendingMcpApproval = $state<{
        request_id: string;
        server: string;
        tool: string;
        arguments: Record<string, unknown>;
    } | null>(null);

    // ── MCP sidebar panel state ────────────────────────────────
    mcpConfigs = $state<McpServerConfig[]>([]);
    mcpConnections = $state<McpConnectionInfo[]>([]);
    mcpExpanded = $state(true);

    #unlisten: (() => void) | null = null;
    #observer: MutationObserver | null = null;
    #chatItemCache: ChatItem[] = [];
    #eventLogTrimmed = 0;
    #pullPromise: Promise<void> | null = null;
    #pullQueued = false;
    #pinIdCounter = 0;
    #PIN_STORAGE_KEY = "workflow-ui:pinned";

    chatItems: ChatItem[] = $derived.by(() => {
        let lastTextIdx = -1;
        for (let i = this.messages.length - 1; i >= 0; i--) {
            if (this.messages[i].type === "text") {
                lastTextIdx = i;
                break;
            }
        }
        const isStreaming = lastTextIdx >= 0 && this.running;
        const prev = this.#chatItemCache;
        const next: ChatItem[] = [];
        let changed = prev.length !== this.messages.length;

        for (let i = 0; i < this.messages.length; i++) {
            const m = this.messages[i];
            if (m.type === "text") {
                const streaming = isStreaming && i === lastTextIdx;
                const cached = prev[i];
                if (
                    !changed &&
                    cached?.type === "assistant" &&
                    cached.text === m.text &&
                    cached.streaming === streaming
                ) {
                    next.push(cached);
                } else {
                    changed = true;
                    next.push({ id: i, type: "assistant", text: m.text, streaming });
                }
            } else if (m.type === "tool") {
                const item = {
                    id: i,
                    type: "tool" as const,
                    text: m.text,
                    result: m.result,
                    status: (m.result ? "done" : "running") as "done" | "running",
                };
                const cached = prev[i];
                if (
                    !changed &&
                    cached?.type === "tool" &&
                    cached.text === m.text &&
                    cached.result === m.result &&
                    cached.status === item.status
                ) {
                    next.push(cached);
                } else {
                    changed = true;
                    next.push(item);
                }
            } else {
                const cached = prev[i];
                if (!changed && cached?.type === m.type && cached.text === m.text) {
                    next.push(cached);
                } else {
                    changed = true;
                    next.push({ id: i, type: m.type, text: m.text });
                }
            }
        }

        this.#chatItemCache = next;
        return next;
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

    stop = async () => {
        if (this.selected == null) return;
        try {
            await invoke("stop_agent", { target: this.selected });
            this.running = false;
        } catch (e) {
            this.error = `stop: ${e}`;
        }
    };

    dismissError = () => {
        this.error = "";
        if (this.errorTimer) {
            clearTimeout(this.errorTimer);
            this.errorTimer = null;
        }
    };

    setError = (msg: string) => {
        this.error = msg;
        if (this.errorTimer) clearTimeout(this.errorTimer);
        this.errorTimer = setTimeout(() => { this.error = ""; }, 8000);
    };

    openDialog = (id: DialogId) => {
        this.dialog = id;
    };
    toggleProject = () => {
        this.projectExpanded = !this.projectExpanded;
    };

    closeDialog = () => {
        this.dialog = null;
    };
    toggleRoles = () => {
        this.rolesExpanded = !this.rolesExpanded;
    };

    toggleMcp = () => {
        this.mcpExpanded = !this.mcpExpanded;
    };

    pinMessage = (item: ChatItem) => {
        const alreadyPinned = this.pinnedMessages.some(p =>
            p.text === item.text &&
            p.type === item.type &&
            (p.result ?? null) === (item.result ?? null)
        );
        if (alreadyPinned) return;
        const agent = this.agents.find(a => a.id === this.selected);
        this.pinnedMessages = [
            ...this.pinnedMessages,
            {
                id: this.#pinIdCounter++,
                chatItemId: item.id,
                text: item.text,
                type: item.type,
                result: item.result,
                status: item.status,
                timestamp: Date.now(),
                agentId: this.selected,
                agentRole: agent ? agent.role : undefined,
            },
        ];
        this.#savePinnedMessages();
    };

    unpinMessage = (pinId: number) => {
        this.pinnedMessages = this.pinnedMessages.filter(p => p.id !== pinId);
        this.#savePinnedMessages();
    };

    togglePinMessage = (item: ChatItem) => {
        const existing = this.pinnedMessages.find(p =>
            p.text === item.text &&
            p.type === item.type &&
            (p.result ?? null) === (item.result ?? null)
        );
        if (existing) {
            this.unpinMessage(existing.id);
        } else {
            this.pinMessage(item);
        }
    };

    #savePinnedMessages = () => {
        try {
            localStorage.setItem(this.#PIN_STORAGE_KEY, JSON.stringify(this.pinnedMessages));
        } catch { /* ignore */ }
    };

    #loadPinnedMessages = () => {
        try {
            const saved = localStorage.getItem(this.#PIN_STORAGE_KEY);
            if (saved) {
                const parsed = JSON.parse(saved) as PinnedMessage[];
                this.pinnedMessages = parsed;
                // restore counter past any existing ids
                const maxId = parsed.reduce((max, p) => Math.max(max, p.id), -1);
                this.#pinIdCounter = maxId + 1;
            }
        } catch { /* ignore */ }
    };

    // ── MCP panel methods ─────────────────────────────────────
    loadMcpConnections = async () => {
        try {
            this.mcpConnections = (await invoke("list_mcp_connections")) as McpConnectionInfo[];
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

    addMcpServer = async (config: McpServerConfig) => {
        try {
            await invoke("add_mcp_server", { config });
            await this.loadMcpConfigs();
            await this.loadMcpConnections();
        } catch (e) {
            this.error = `add mcp server: ${e}`;
        }
    };

    removeMcpServer = async (name: string) => {
        try {
            await invoke("remove_mcp_server", { name });
            await this.loadMcpConfigs();
            await this.loadMcpConnections();
        } catch (e) {
            this.error = `remove mcp server: ${e}`;
        }
    };

    loadRoles = async () => {
        try {
            this.roles = (await invoke("load_roles")) as RoleInfo[];
        } catch (e) {
            this.error = `load roles: ${e}`;
        }
    };

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

    pull = async (sel?: AgentId | null) => {
        // Serialize: if a pull is already in-flight, queue a retry.
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
                        !this.agents.find(a => a.id === this.selected)?.current_task
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

    addRole = async (name: string, def: string) => {
        if (!name.trim() || !def.trim()) return;
        this.pendingAction = { type: "add-role" };
        try {
            this.roles = (await invoke("add_role", {
                name: name.trim(),
                definition: def.trim(),
            })) as RoleInfo[];
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

    loadProviders = async () => {
        try {
            this.providers = (await invoke(
                "list_providers",
            )) as ProviderEntry[];
        } catch (e) {
            this.error = `load providers: ${e}`;
        }
    };

    refreshProviders = async () => {
        this.pendingAction = { type: "refresh-providers" };
        try {
            this.providers = (await invoke(
                "fetch_providers",
            )) as ProviderEntry[];
            this.error = "";
        } catch (e) {
            this.error =
                e instanceof Error ? e.message : `refresh providers: ${e}`;
        } finally {
            this.pendingAction = null;
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

    reconfigureProject = async (newPath: string) => {
        // Reconfigure with existing provider settings but new project path.
        try {
            await invoke("configure_runtime", {
                providerId: this.selectedProvider,
                apiKey: this.settingsApiKey,
                model: this.selectedModel,
                projectPath: newPath,
            });
            this.configured = true;
            this.error = "";
            this.roles = (await invoke("load_roles")) as RoleInfo[];
            await this.refreshProject();
            await this.pull(null);
        } catch (e) {
            this.error = `project: ${e}`;
            throw e; // re-throw so UI shows the error in the panel
        }
    };

    clearProject = async () => {
        // Reconfigure without a project.
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
            this.roles = (await invoke("load_roles")) as RoleInfo[];
            await this.refreshProject();
            await this.pull(null);
            await this.saveUserConfig();
        } catch (e) {
            this.error = `configure: ${e}`;
            throw e;
        }
    };

    init = () => {
        // Initialize shiki highlighter with dual-theme support
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
        this.#loadPinnedMessages();

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

        listen<UiEvent>("workflow:event", (event) => {
            try {
                const entry: LogEntry = { ts: Date.now(), event: event.payload };
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
                    const idx = this.mcpServers.findIndex((s) => s.name === server);
                    if (idx >= 0) {
                        this.mcpServers[idx] = { name: server, tool_count };
                    } else {
                        this.mcpServers.push({ name: server, tool_count });
                    }
                    this.loadMcpConnections();
                }
                if (event.payload.type === "mcp_disconnected") {
                    const msg: { type: "mcp_disconnected"; server: string } = event.payload as any;
                    this.mcpServers = this.mcpServers.filter(
                        (s) => s.name !== msg.server,
                    );
                    this.loadMcpConnections();
                }
                if (event.payload.type === "mcp_tool_needs_approval") {
                    this.pendingMcpApproval = event.payload;
                    this.dialog = "mcp-approval";
                }
                this.pull();
            } catch (e) {
                console.error("event handler:", e);
            }
        }).then((unlisten) => {
            this.#unlisten = unlisten;
        }).catch((e) => {
            console.error("failed to listen for events:", e);
        });
    };

    destroy = () => {
        this.#unlisten?.();
        this.#observer?.disconnect();
        this.#observer = null;
        if (this.errorTimer) {
            clearTimeout(this.errorTimer);
            this.errorTimer = null;
        }
        this.#chatItemCache = [];
        this.eventLog = [];
        this.messages = [];
        this.agents = [];
    };
}

export const state = new AppState();
