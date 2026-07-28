import type { AgentId, AgentInfo, ChatItem, ConversationMessage, LogEntry, PinnedMessage } from "../types";

export class ChatStore {
	messages = $state<ConversationMessage[]>([]);
	input = $state("");
	running = $state(false);
	pinnedMessages = $state<PinnedMessage[]>([]);
	eventLog = $state<LogEntry[]>([]);

	#chatItemCache: ChatItem[] = [];
	#eventLogTrimmed = 0;
	#pinIdCounter = 0;
	#PIN_STORAGE_KEY = "workflow-ui:pinned";
	#lastStreamingText = "";

	chatItems: ChatItem[] = $derived.by(() => {
		let lastTextIdx = -1;
		for (let i = this.messages.length - 1; i >= 0; i--) {
			if (this.messages[i].type === "text") {
				lastTextIdx = i;
				break;
			}
		}
		const lastText = lastTextIdx >= 0 ? this.messages[lastTextIdx].text : "";
		// Only mark as streaming when text content is actively changing.
		// This avoids the SvelteMarkdown bug where toggling streaming on an
		// already-complete message clears its content.
		const textChanged = lastText !== this.#lastStreamingText;
		this.#lastStreamingText = lastText;
		const isStreaming = lastTextIdx >= 0 && this.running && textChanged;
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
				const toolStatus = m.is_error
					? ("error" as const)
					: m.result
						? ("done" as const)
						: ("running" as const);
				const item = {
					id: i,
					type: "tool" as const,
					text: m.text,
					result: m.result,
					status: toolStatus,
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

	pinMessage = (item: ChatItem, agents: AgentInfo[], selected: AgentId | null) => {
		const alreadyPinned = this.pinnedMessages.some(
			(p) =>
				p.text === item.text &&
				p.type === item.type &&
				(p.result ?? null) === (item.result ?? null),
		);
		if (alreadyPinned) return;
		const agent = agents.find((a) => a.id === selected);
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
				agentId: selected,
				agentRole: agent ? agent.role : undefined,
			},
		];
		this.#savePinnedMessages();
	};

	unpinMessage = (pinId: number) => {
		this.pinnedMessages = this.pinnedMessages.filter((p) => p.id !== pinId);
		this.#savePinnedMessages();
	};

	togglePinMessage = (item: ChatItem, agents: AgentInfo[], selected: AgentId | null) => {
		const existing = this.pinnedMessages.find(
			(p) =>
				p.text === item.text &&
				p.type === item.type &&
				(p.result ?? null) === (item.result ?? null),
		);
		if (existing) {
			this.unpinMessage(existing.id);
		} else {
			this.pinMessage(item, agents, selected);
		}
	};

	#savePinnedMessages = () => {
		try {
			localStorage.setItem(this.#PIN_STORAGE_KEY, JSON.stringify(this.pinnedMessages));
		} catch {
			/* ignore */
		}
	};

	loadPinnedMessages = () => {
		try {
			const saved = localStorage.getItem(this.#PIN_STORAGE_KEY);
			if (saved) {
				const parsed = JSON.parse(saved) as PinnedMessage[];
				this.pinnedMessages = parsed;
				const maxId = parsed.reduce((max, p) => Math.max(max, p.id), -1);
				this.#pinIdCounter = maxId + 1;
			}
		} catch {
			/* ignore */
		}
	};
}
