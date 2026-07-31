<script lang="ts">
	import { Plus, Layers, Trash2, Check, Pencil, X, Folder, FolderX } from "@lucide/svelte";
	import { invoke } from "@tauri-apps/api/core";
	import { Button } from "$lib/components/ui/button";
	import * as Popover from "$lib/components/ui/popover";
	import { cn } from "$lib/utils";
	import type { SessionMeta } from "$lib/types";

	let {
		sessions,
		activeId,
		defaultName = "",
		onCreate,
		onSwitch,
		onDelete,
		onRename,
		onSetProject,
	}: {
		sessions: SessionMeta[];
		activeId: number | null;
		defaultName?: string;
		onCreate: (name: string) => void;
		onSwitch: (id: number) => void;
		onDelete: (id: number) => void;
		onRename: (id: number, name: string) => Promise<boolean>;
		onSetProject?: (id: number, projectPath: string | null) => void;
	} = $props();

	let open = $state(false);
	let renamingId = $state<number | null>(null);
	let renameValue = $state("");

	const activeName = $derived(
		sessions.find((s) => s.id === activeId)?.name ?? "No session",
	);

	function nextName(): string {
		// Name the session after the opened project folder when available.
		if (defaultName) {
			let candidate = defaultName;
			let i = 2;
			while (sessions.some((s) => s.name === candidate)) {
				candidate = `${defaultName} ${i++}`;
			}
			return candidate;
		}
		const prefix = "Session";
		const nums = sessions
			.map((s) => {
				const m = s.name.match(/^Session (\d+)$/);
				return m ? parseInt(m[1], 10) : 0;
			})
			.filter((n) => n > 0);
		const max = nums.length > 0 ? Math.max(...nums) : 0;
		return `${prefix} ${max + 1}`;
	}

	function handleCreate() {
		onCreate(nextName());
		open = false;
	}

	function handleSelect(id: number) {
		if (renamingId !== null) return;
		if (id !== activeId) {
			onSwitch(id);
		}
		open = false;
	}

	function startRename(e: Event, id: number, name: string) {
		e.stopPropagation();
		renamingId = id;
		renameValue = name;
	}

	async function confirmRename() {
		if (renamingId == null || !renameValue.trim()) return;
		await onRename(renamingId, renameValue.trim());
		renamingId = null;
	}

	function cancelRename() {
		renamingId = null;
	}

	function handleDelete(e: Event, id: number) {
		e.stopPropagation();
		onDelete(id);
	}

	async function handleSetProject(e: Event, id: number) {
		e.stopPropagation();
		const picked = await invoke<string | null>("pick_folder");
		// Only bind when a folder was actually picked; a cancelled picker
		// leaves the current binding untouched.
		if (picked) onSetProject?.(id, picked);
	}

	function projectLabel(s: SessionMeta): string {
		if (!s.project) return "No project";
		const parts = s.project.split(/[\\/]/);
		return parts[parts.length - 1] || s.project;
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger>
		{#snippet child({ props }: { props: Record<string, unknown> })}
			<Button
				{...props}
				variant="ghost"
				size="sm"
				class="text-xs font-medium text-muted-foreground/70 hover:text-foreground gap-1.5 px-2"
			>
				<Layers class="size-3" />
				<span class="max-w-24 truncate">{activeName}</span>
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="min-w-48 p-1.5" side="bottom" align="start">
		<div class="flex flex-col gap-0.5">
			{#each sessions as s (s.id)}
				<div class="group relative">
					{#if renamingId === s.id}
						<div class="flex items-center gap-1 px-2 py-1.5">
							<input
								type="text"
								bind:value={renameValue}
								class="flex-1 min-w-0 bg-muted/20 rounded px-1.5 py-0.5 text-xs outline-none focus:ring-1 focus:ring-ring border border-border/30"
								autofocus
								onkeydown={(e) => {
									if (e.key === "Enter") confirmRename();
									if (e.key === "Escape") cancelRename();
								}}
							/>
							<button
								class="size-4 flex items-center justify-center rounded hover:text-emerald-500 hover:bg-emerald-500/10 disabled:opacity-20 transition-colors"
								disabled={!renameValue.trim()}
								onclick={confirmRename}
								title="Save"
							>
								<Check class="size-3" />
							</button>
							<button
								class="size-4 flex items-center justify-center rounded hover:text-muted-foreground hover:bg-muted/30 transition-colors"
								onclick={cancelRename}
								title="Cancel"
							>
								<X class="size-3" />
							</button>
						</div>
					{:else}
						<button
							class={cn(
								"w-full flex items-center gap-2 px-2 py-1.5 rounded text-xs text-left transition-colors",
								s.id === activeId
									? "bg-accent text-accent-foreground font-medium"
									: "text-muted-foreground hover:bg-muted/30 hover:text-foreground",
							)}
							onclick={() => handleSelect(s.id)}
						>
							<Layers class="size-3 shrink-0 opacity-60" />
							<span class="flex-1 min-w-0">
								<span class="block truncate">{s.name}</span>
								<span class="block truncate text-[10px] opacity-60">
									{s.project ? projectLabel(s) : "No project"}
								</span>
							</span>
							<span
								class="size-4 flex items-center justify-center rounded opacity-0 group-hover:opacity-100 hover:bg-muted/50 transition-all text-muted-foreground/40 hover:text-foreground cursor-pointer"
								onclick={(e) => handleSetProject(e, s.id)}
								onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleSetProject(e, s.id); } }}
								role="button"
								tabindex="0"
								title={s.project ? "Change project folder" : "Set project folder"}
							>
								{#if s.project}
									<Folder class="size-2.5 text-primary" />
								{:else}
									<Folder class="size-2.5" />
								{/if}
							</span>
							<span
								class="size-4 flex items-center justify-center rounded opacity-0 group-hover:opacity-100 hover:bg-muted/50 transition-all text-muted-foreground/40 hover:text-foreground cursor-pointer"
								onclick={(e) => { e.stopPropagation(); startRename(e, s.id, s.name); }}
								onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); startRename(e, s.id, s.name); } }}
								role="button"
								tabindex="0"
								title="Rename"
							>
								<Pencil class="size-2.5" />
							</span>
							<span
								class="size-4 flex items-center justify-center rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 hover:text-red-500 transition-all text-muted-foreground/40 cursor-pointer"
								onclick={(e) => { e.stopPropagation(); handleDelete(e, s.id); }}
								onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleDelete(e, s.id); } }}
								role="button"
								tabindex="0"
								title="Delete"
							>
								<Trash2 class="size-2.5" />
							</span>
						</button>
					{/if}
				</div>
			{:else}
				<div class="py-6 text-center text-[11px] text-muted-foreground/40">
					No sessions
				</div>
			{/each}
		</div>

		<div class="mt-1 pt-1 border-t border-border/40">
			<button
				class="w-full flex items-center gap-2 px-2 py-1.5 rounded text-xs text-muted-foreground hover:text-foreground hover:bg-muted/30 transition-colors"
				onclick={handleCreate}
			>
				<Plus class="size-3" />
				<span>New Session</span>
			</button>
		</div>
	</Popover.Content>
</Popover.Root>
