import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initHighlighter, setTheme } from "./markdown/highlighter";

// shiki language and theme imports
import js from "shiki/langs/javascript.mjs";
import ts from "shiki/langs/typescript.mjs";
import py from "shiki/langs/python.mjs";
import rs from "shiki/langs/rust.mjs";
import json from "shiki/langs/json.mjs";
import html from "shiki/langs/html.mjs";
import css from "shiki/langs/css.mjs";
import shellscript from "shiki/langs/shellscript.mjs";
import sql from "shiki/langs/sql.mjs";
import md from "shiki/langs/markdown.mjs";
import yaml from "shiki/langs/yaml.mjs";
import xml from "shiki/langs/xml.mjs";
import toml from "shiki/langs/toml.mjs";
import go from "shiki/langs/go.mjs";
import rb from "shiki/langs/ruby.mjs";
import java from "shiki/langs/java.mjs";
import c from "shiki/langs/c.mjs";
import cpp from "shiki/langs/cpp.mjs";
import php from "shiki/langs/php.mjs";
import diff from "shiki/langs/diff.mjs";
import graphql from "shiki/langs/graphql.mjs";
import ini from "shiki/langs/ini.mjs";
import kt from "shiki/langs/kotlin.mjs";
import lua from "shiki/langs/lua.mjs";
import make from "shiki/langs/make.mjs";
import perl from "shiki/langs/perl.mjs";
import r from "shiki/langs/r.mjs";
import scala from "shiki/langs/scala.mjs";
import swift from "shiki/langs/swift.mjs";
import svelte from "shiki/langs/svelte.mjs";
import docker from "shiki/langs/docker.mjs";
import solidity from "shiki/langs/solidity.mjs";
import zig from "shiki/langs/zig.mjs";

import githubDark from "shiki/themes/github-dark-default.mjs";
import githubLight from "shiki/themes/github-light-default.mjs";

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
    running = $state(false);

    eventLog = $state<LogEntry[]>([]);

    #unlisten: (() => void) | null = null;
    #chatItemCache: ChatItem[] = [];

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

    openDialog = (id: DialogId) => {
        this.dialog = id;
    };
    closeDialog = () => {
        this.dialog = null;
    };
    toggleRoles = () => {
        this.rolesExpanded = !this.rolesExpanded;
    };

    loadRoles = async () => {
        try {
            this.roles = (await invoke("load_roles")) as RoleInfo[];
        } catch (e) {
            this.error = `load roles: ${e}`;
        }
    };

    pull = async (sel?: AgentId | null) => {
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
            } | null;
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

    configureRuntime = async (
        providerId: string,
        apiKey: string,
        model: string,
    ) => {
        try {
            await invoke("configure_runtime", { providerId, apiKey, model });
            this.selectedProvider = providerId;
            this.selectedModel = model;
            this.settingsApiKey = apiKey;
            this.configured = true;
            this.error = "";
            this.closeDialog();
            this.roles = (await invoke("load_roles")) as RoleInfo[];
            await this.pull(null);
            await this.saveUserConfig();
        } catch (e) {
            this.error = `configure: ${e}`;
        }
    };

    init = () => {
        // Initialize shiki highlighter with dual-theme support
        try {
            const langs = [
                js, ts, py, rs, json, html, css, shellscript, sql, md,
                yaml, xml, toml, go, rb, java, c, cpp, php, diff, graphql,
                ini, kt, lua, make, perl, r, scala, swift, svelte, docker,
                solidity, zig,
            ].flat();
            const themes = [githubDark, githubLight];
            const isDark = document.documentElement.classList.contains("dark");
            initHighlighter(langs, themes, isDark ? "github-dark-default" : "github-light-default");
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
                );
            }
        });
        this.loadRoles();
        this.pull(null);
        this.loadProviders();

        const updateTheme = () => {
            const isDark = document.documentElement.classList.contains("dark");
            const theme = isDark ? "github-dark-default" : "github-light-default";
            try {
                setTheme(theme);
            } catch {
                // highlighter may not be ready yet
            }
        };
        updateTheme();
        const observer = new MutationObserver(updateTheme);
        observer.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["class"],
        });

        listen<UiEvent>("workflow:event", (event) => {
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
