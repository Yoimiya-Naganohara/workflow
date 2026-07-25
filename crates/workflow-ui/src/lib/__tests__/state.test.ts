import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(() => Promise.resolve(() => {})),
}));

const { state } = await import("$lib/state.svelte.js");

describe("state", () => {
	beforeEach(() => {
		state.agents.length = 0;
		state.messages.length = 0;
		state.error = "";
		state.selected = null;
		state.pendingAction = null;
		state.input = "";
	});

	it("chatItems maps ConversationMessage to ChatItem correctly", () => {
		state.messages.push(
			{ type: "user", text: "hello" },
			{ type: "text", text: "world" },
			{ type: "thinking", text: "hmm" },
			{ type: "tool", text: "search", result: null },
			{ type: "error", text: "fail" },
		);

		expect(state.chatItems).toHaveLength(5);
		expect(state.chatItems[0]).toMatchObject({ type: "user", text: "hello" });
		expect(state.chatItems[1]).toMatchObject({ type: "assistant", text: "world" });
		expect(state.chatItems[2]).toMatchObject({ type: "thinking", text: "hmm" });
		expect(state.chatItems[3]).toMatchObject({ type: "tool", text: "search", status: "running" });
		expect(state.chatItems[4]).toMatchObject({ type: "error", text: "fail" });
	});

	it("chatItems sets tool status done when result is present", () => {
		state.messages.push({ type: "tool", text: "search", result: '{"ok":true}' });
		expect(state.chatItems[0]).toMatchObject({ type: "tool", status: "done" });
	});

	it("agentStatuses returns idle for agents without current_task", () => {
		state.agents.push(
			{ id: 1, role: "planner", current_task: null, state: "idle" },
			{ id: 2, role: "executor", current_task: "build", state: "thinking" },
		);
		const statuses = state.agentStatuses;
		expect(statuses.get(1)).toBe("idle");
		expect(statuses.get(2)).toBe("thinking");
	});

	it("agentStatuses reflects last message state", () => {
		state.selected = 1;
		state.agents.push({ id: 1, role: "planner", current_task: null, state: "idle" });
		state.messages.push({ type: "tool", text: "search", result: null });

		expect(state.agentStatuses.get(1)).toBe("running-tool");
	});

	it("openDialog and closeDialog manage dialog state", () => {
		expect(state.dialog).toBeNull();
		state.openDialog("settings");
		expect(state.dialog).toBe("settings");
		state.closeDialog();
		expect(state.dialog).toBeNull();
	});

	it("toggleRoles flips rolesExpanded", () => {
		expect(state.rolesExpanded).toBe(false);
		state.toggleRoles();
		expect(state.rolesExpanded).toBe(true);
		state.toggleRoles();
		expect(state.rolesExpanded).toBe(false);
	});

	it("submit does nothing when input is empty", async () => {
		state.input = "";
		state.selected = 1;
		await state.submit();
		expect(state.error).toBe("");
	});

	it("submit returns early when no agent selected", async () => {
		state.input = "hello";
		state.selected = null;
		await state.submit();
		expect(state.input).toBe("hello");
	});

	it("submit calls invoke with correct parameters", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue({
			agents: [{ id: 1, role: "planner", current_task: null }],
			selected: 1,
			messages: [],
		});

		state.selected = 1;
		state.input = "hello world";
		await state.submit();

		expect(invoke).toHaveBeenCalledWith("send", { target: 1, text: "hello world" });
		expect(state.input).toBe("");
		expect(state.pendingAction).toBeNull();
	});
});
