<script lang="ts">
	import * as Dialog from "$lib/components/ui/dialog";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { Textarea } from "$lib/components/ui/textarea";
	import { Card } from "$lib/components/ui/card";
	import { Brain, Plus } from "@lucide/svelte";
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

	function handleAdd() {
		if (!name.trim() || !def.trim()) return;
		onAddRole(name.trim(), def.trim());
		name = "";
		def = "";
	}
</script>

<Dialog.Root {open} onOpenChange={onOpenChange}>
<Dialog.Content class="sm:max-w-lg">
	<Dialog.Header>
		<Dialog.Title>Roles</Dialog.Title>
		<Dialog.Description>Manage agent roles and their system prompts.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-4 max-h-64 overflow-y-auto">
		{#each roles as r}
			<Card class="p-3 space-y-1">
				<div class="flex items-center gap-2">
					<Brain class="size-3.5 text-muted-foreground" />
					<span class="text-sm font-medium">{r.name}</span>
				</div>
				<p class="text-xs text-muted-foreground">{r.definition}</p>
			</Card>
		{/each}
	</div>
	<div class="border-t border-border pt-3 space-y-3">
		<p class="text-xs font-medium text-muted-foreground">Add New Role</p>
		<Input bind:value={name} placeholder="Role name" />
		<Textarea bind:value={def} placeholder="System prompt / definition..." class="min-h-20 resize-none" />
		<Button onclick={handleAdd} class="w-full" size="sm">
			<Plus class="size-3.5" /> Add Role
		</Button>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => onOpenChange(false)}>Close</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
