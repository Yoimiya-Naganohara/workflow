import { invoke } from "@tauri-apps/api/core";
import type { SessionMeta } from "$lib/types";

export class SessionStore {
	sessions: SessionMeta[] = $state([]);
	activeId: number | null = $state(null);

	get activeName(): string {
		const s = this.sessions.find((s) => s.id === this.activeId);
		return s?.name ?? "";
	}

	loadSessions = async () => {
		try {
			this.sessions = await invoke<SessionMeta[]>("list_sessions");
			const active = await invoke<SessionMeta | null>("get_active_session");
			this.activeId = active?.id ?? null;
		} catch (e) {
			console.error("load sessions:", e);
		}
	};

	createSession = async (name: string): Promise<SessionMeta | null> => {
		try {
			const meta = await invoke<SessionMeta>("create_session", { name });
			this.sessions = await invoke<SessionMeta[]>("list_sessions");
			this.activeId = meta.id;
			return meta;
		} catch (e) {
			console.error("create session:", e);
			return null;
		}
	};

	switchSession = async (id: number) => {
		try {
			await invoke<SessionMeta>("switch_session", { id });
			this.activeId = id;
			// Refresh ordering (most recently used first).
			this.sessions = await invoke<SessionMeta[]>("list_sessions");
		} catch (e) {
			console.error("switch session:", e);
		}
	};

	deleteSession = async (id: number) => {
		try {
			await invoke("delete_session", { id });
			// The backend keeps at least one active session (it activates the
			// most recently used remaining one, or creates a fresh one when
			// none are left), so re-sync instead of picking locally.
			this.sessions = await invoke<SessionMeta[]>("list_sessions");
			const active = await invoke<SessionMeta | null>("get_active_session");
			this.activeId = active?.id ?? null;
		} catch (e) {
			console.error("delete session:", e);
		}
	};

	renameSession = async (id: number, name: string): Promise<boolean> => {
		try {
			await invoke<SessionMeta>("rename_session", { id, name });
			const s = this.sessions.find((s) => s.id === id);
			if (s) s.name = name;
			return true;
		} catch (e) {
			console.error("rename session:", e);
			return false;
		}
	};

	bindSessionProject = async (id: number, projectPath: string | null): Promise<boolean> => {
		try {
			const meta = await invoke<SessionMeta>("bind_session_project", {
				sessionId: id,
				projectPath,
			});
			const s = this.sessions.find((s) => s.id === id);
			if (s) {
				s.name = meta.name;
				s.project = meta.project;
			}
			return true;
		} catch (e) {
			console.error("bind session project:", e);
			return false;
		}
	};

	saveSessions = async () => {
		try {
			await invoke<number>("save_sessions");
		} catch (e) {
			console.error("save sessions:", e);
		}
	};
}
