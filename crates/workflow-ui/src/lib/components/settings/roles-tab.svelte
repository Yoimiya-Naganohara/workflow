<script lang="ts">
	import { Plus, Trash2, Check, X, User } from "@lucide/svelte";
	import { cn } from "$lib/utils";
	import type { RoleInfo } from "$lib/types";

	let {
		roles = [],
		onAddRole,
		onDeleteRole,
	}: {
		roles?: RoleInfo[];
		onAddRole: (name: string, def: string) => void;
		onDeleteRole: (name: string) => void;
	} = $props();

	let editingId = $state<string | null>(null);
	let editingName = $state("");
	let editingDef = $state("");
	let creatingNew = $state(false);

	const editingRole = $derived(
		creatingNew
			? null
			: editingId
				? roles.find((r) => r.id === editingId)
				: null,
	);

	function startCreate() {
		if (editingId && editingName.trim() && editingDef.trim()) {
			onAddRole(editingName.trim(), editingDef.trim());
		}
		editingId = null;
		creatingNew = true;
		editingName = "";
		editingDef = "";
	}

	function confirmNew() {
		if (!editingName.trim() || !editingDef.trim()) return;
		onAddRole(editingName.trim(), editingDef.trim());
		creatingNew = false;
		editingName = "";
		editingDef = "";
	}

	function cancelNew() {
		creatingNew = false;
		editingName = "";
		editingDef = "";
	}

	function startEdit(r: RoleInfo) {
		creatingNew = false;
		editingId = r.id;
		editingName = r.name;
		editingDef = r.definition;
	}

	function saveEdit() {
		if (!editingId || !editingName.trim() || !editingDef.trim()) return;
		onAddRole(editingName.trim(), editingDef.trim());
	}

	async function handleDelete(name: string) {
		await onDeleteRole(name);
		editingId = null;
		editingName = "";
		editingDef = "";
	}

	function cancelEdit() {
		editingId = null;
		creatingNew = false;
	}
</script>

<div class="flex flex-col h-full">
	<div class="flex items-center justify-between mb-2 shrink-0">
		<div>
			<h2 class="text-sm font-semibold">Roles</h2>
			<p class="text-[11px] text-muted-foreground/50">
				{roles.length} role{roles.length !== 1 ? "s" : ""}
			</p>
		</div>
		<button
			class="flex items-center gap-1 px-2 py-1 rounded text-[11px] font-medium text-muted-foreground hover:text-foreground hover:bg-muted/40 transition-colors"
			onclick={startCreate}
			title="New role"
		>
			<Plus class="size-3" />
			New
		</button>
	</div>

	<div class="flex flex-1 min-h-0 gap-2">
		<!-- Role list -->
		<div class="w-44 shrink-0 overflow-y-auto -mx-1 px-1 space-y-px">
			{#each roles as r (r.id)}
				<div class="group relative">
					<button
						class={cn(
							"w-full flex items-center gap-1.5 px-2 py-1 rounded text-xs text-left transition-colors",
							editingId === r.id
								? "bg-accent text-accent-foreground font-medium"
								: "text-muted-foreground hover:bg-muted/30 hover:text-foreground",
						)}
						onclick={() => {
							if (editingId !== r.id) {
								if (editingId && editingName.trim() && editingDef.trim()) {
									onAddRole(editingName.trim(), editingDef.trim());
								}
								startEdit(r);
							}
						}}
					>
						<span class="truncate flex-1 min-w-0">{r.name}</span>
					</button>
					<button
						class="absolute right-1 top-1/2 -translate-y-1/2 size-4 flex items-center justify-center rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 hover:text-red-500 transition-all text-muted-foreground/40"
						onclick={(e) => { e.stopPropagation(); handleDelete(r.id); }}
						title="Delete"
					>
						<Trash2 class="size-2.5" />
					</button>
				</div>
			{:else}
				<div class="py-8 text-center text-[11px] text-muted-foreground/40">No roles</div>
			{/each}
		</div>

		<!-- Editor panel -->
		<div class="flex-1 min-w-0 rounded-md border border-border/40 bg-card flex flex-col min-h-0">
			{#if creatingNew}
				<div class="flex items-center gap-1.5 px-3 py-2 border-b border-border/40 shrink-0">
					<Plus class="size-3 text-muted-foreground/50 shrink-0" />
					<span class="text-xs font-medium text-muted-foreground/70">New Role</span>
					<div class="flex-1"></div>
					<button class="size-5 flex items-center justify-center rounded hover:text-emerald-500 hover:bg-emerald-500/10 disabled:opacity-20 transition-colors" onclick={confirmNew} disabled={!editingName.trim() || !editingDef.trim()} title="Create">
						<Check class="size-3" />
					</button>
					<button class="size-5 flex items-center justify-center rounded hover:text-muted-foreground hover:bg-muted/30 transition-colors" onclick={cancelNew} title="Cancel">
						<X class="size-3" />
					</button>
				</div>
				<div class="flex-1 p-3 min-h-0">
					<div class="flex flex-col h-full gap-2">
						<!-- svelte-ignore a11y_autofocus -->
						<input type="text" bind:value={editingName} placeholder="Role name" class="w-full bg-muted/20 rounded px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-ring border border-border/30" autofocus />
						<textarea bind:value={editingDef} placeholder="System prompt / definition..." class="flex-1 min-h-0 resize-none bg-muted/20 rounded px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-ring border border-border/30 font-mono leading-relaxed"></textarea>
					</div>
				</div>
			{:else if editingRole}
				<div class="flex items-center gap-1.5 px-3 py-2 border-b border-border/40 shrink-0">
					<span class="text-[11px] text-muted-foreground/50 font-medium">Edit</span>
					<div class="flex-1"></div>
					<button class="size-5 flex items-center justify-center rounded hover:text-emerald-500 hover:bg-emerald-500/10 disabled:opacity-20 transition-colors" onclick={saveEdit} disabled={!editingName.trim() || !editingDef.trim()} title="Save">
						<Check class="size-3" />
					</button>
					<button class="size-5 flex items-center justify-center rounded hover:text-muted-foreground hover:bg-muted/30 transition-colors" onclick={cancelEdit} title="Cancel">
						<X class="size-3" />
					</button>
				</div>
				<div class="flex-1 p-3 min-h-0">
					<div class="flex flex-col h-full gap-2">
						<input type="text" bind:value={editingName} placeholder="Role name" class="w-full bg-muted/20 rounded px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-ring border border-border/30" />
						<textarea bind:value={editingDef} placeholder="System prompt / definition..." class="flex-1 min-h-0 resize-none bg-muted/20 rounded px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-ring border border-border/30 font-mono leading-relaxed"></textarea>
					</div>
				</div>
			{:else}
				<div class="flex flex-col items-center justify-center h-full text-center px-4">
					<User class="size-7 text-muted-foreground/15 mb-2" />
					<p class="text-[11px] text-muted-foreground/30">Select a role or create new</p>
				</div>
			{/if}
		</div>
	</div>
</div>
