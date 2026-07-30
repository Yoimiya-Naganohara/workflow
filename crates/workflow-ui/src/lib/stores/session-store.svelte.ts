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
		} catch (e) {
			console.error("switch session:", e);
		}
	};

	deleteSession = async (id: number) => {
		try {
			await invoke("delete_session", { id });
			this.sessions = this.sessions.filter((s) => s.id !== id);
			if (this.activeId === id) {
				this.activeId = this.sessions[0]?.id ?? null;
			}
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

	saveSessions = async () => {
		try {
			await invoke<number>("save_sessions");
		} catch (e) {
			console.error("save sessions:", e);
		}
	};
}
