import { invoke } from "@tauri-apps/api/core";
import type { AgentId, AgentInfo, DialogId, PendingAction, RoleInfo } from "../types";

export class AgentStore {
	agents = $state<AgentInfo[]>([]);
	selected = $state<AgentId | null>(null);
	dialog = $state<DialogId | null>(null);
	pendingAction = $state<PendingAction>(null);
	error = $state("");
	errorTimer: ReturnType<typeof setTimeout> | null = null;
	rolesExpanded = $state(false);
	roles = $state<RoleInfo[]>([]);

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
		this.errorTimer = setTimeout(() => {
			this.error = "";
		}, 8000);
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

	addRole = async (name: string, def: string) => {
		if (!name.trim() || !def.trim()) return;
		this.pendingAction = { type: "add-role" };
		try {
			this.roles = (await invoke("add_role", {
				name: name.trim(),
				definition: def.trim(),
			})) as RoleInfo[];
		} catch (e) {
			this.setError(`add role: ${e}`);
		} finally {
			this.pendingAction = null;
		}
	};

	loadRoles = async () => {
		try {
			this.roles = (await invoke("load_roles")) as RoleInfo[];
		} catch (e) {
			this.setError(`load roles: ${e}`);
		}
	};
}
