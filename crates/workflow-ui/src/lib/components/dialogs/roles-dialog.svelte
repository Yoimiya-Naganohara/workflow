<script lang="ts">
	import * as Dialog from "$lib/components/ui/dialog";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { Textarea } from "$lib/components/ui/textarea";
	import { Card } from "$lib/components/ui/card";
	import { Brain, Plus, Pencil, Trash2, Check, X as XIcon } from "@lucide/svelte";
	import type { RoleInfo } from "$lib/types";

	let {
		open,
		roles,
		onAddRole,
		onOpenChange,
	}: {
		open: boolean;
		roles: RoleInfo[];
		onAddRole: (name: string, def: string) => void;
		onOpenChange: (open: boolean) => void;
	} = $props();

	let name = $state("");
	let def = $state("");
	let editingId = $state<string | null>(null);
	let editingName = $state("");
	let editingDef = $state("");

	function handleAdd() {
		if (!name.trim() || !def.trim()) return;
		onAddRole(name.trim(), def.trim());
		name = "";
		def = "";
	}

	function startEdit(r: RoleInfo) {
		editingId = r.id;
		editingName = r.name;
		editingDef = r.definition;
	}

	function cancelEdit() {
		editingId = null;
	}

	function saveEdit() {
		if (!editingId || !editingName.trim() || !editingDef.trim()) return;
		onAddRole(editingName.trim(), editingDef.trim());
		editingId = null;
	}
</script>

<Dialog.Root {open} onOpenChange={onOpenChange}>
<Dialog.Content class="sm:max-w-lg">
	<Dialog.Header>
		<Dialog.Title>Roles</Dialog.Title>
		<Dialog.Description>Manage agent roles and their system prompts.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-4 max-h-64 overflow-y-auto -mx-1 px-1">
		{#each roles as r (r.id)}
			<Card class="p-3 space-y-1.5 group relative">
				{#if editingId === r.id}
					<div class="space-y-2">
						<Input bind:value={editingName} placeholder="Role name" class="text-sm h-8" />
						<Textarea bind:value={editingDef} placeholder="System prompt / definition..." class="min-h-16 resize-none text-xs" />
						<div class="flex items-center gap-1.5 justify-end">
							<Button variant="ghost" size="icon-xs" onclick={cancelEdit} title="Cancel">
								<XIcon class="size-3" />
							</Button>
							<Button variant="ghost" size="icon-xs" onclick={saveEdit} title="Save" class="hover:text-emerald-500">
								<Check class="size-3" />
							</Button>
						</div>
					</div>
				{:else}
					<div class="flex items-start gap-2">
						<Brain class="size-3.5 text-muted-foreground mt-0.5 shrink-0" />
						<div class="flex-1 min-w-0">
							<span class="text-sm font-medium">{r.name}</span>
							<p class="text-xs text-muted-foreground mt-0.5">{r.definition}</p>
						</div>
					</div>
				{/if}
			</Card>
		{/each}
	</div>
	<div class="border-t border-border pt-3 space-y-3">
		<p class="text-xs font-medium text-muted-foreground">Add New Role</p>
		<Input bind:value={name} placeholder="Role name" class="text-xs h-8" />
		<Textarea bind:value={def} placeholder="System prompt / definition..." class="min-h-16 resize-none text-xs" />
		<Button onclick={handleAdd} class="w-full" size="sm">
			<Plus class="size-3.5" /> Add Role
		</Button>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => onOpenChange(false)}>Close</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
