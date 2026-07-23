<script lang="ts">
	import * as Dialog from "$lib/components/ui/dialog";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { Card } from "$lib/components/ui/card";
	import { Bot, Search as SearchIcon } from "@lucide/svelte";
	import type { RoleInfo } from "$lib/types";

	let {
		open,
		roles,
		onCreate,
		onOpenChange,
	}: {
		open: boolean;
		roles: RoleInfo[];
		onCreate: (role: string) => void;
		onOpenChange: (open: boolean) => void;
	} = $props();

	let selectedRole = $state("");
	let searchQuery = $state("");
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen) {
			selectedRole = roles[0]?.name ?? "";
			searchQuery = "";
		}
		wasOpen = open;
	});

	const filteredRoles = $derived(
		searchQuery
			? roles.filter(r => r.name.toLowerCase().includes(searchQuery.toLowerCase()))
			: roles,
	);

	const roleDesc = $derived.by(() => {
		const r = roles.find(r => r.name === selectedRole);
		return r ? r.definition : "";
	});

	const canCreate = $derived(!!selectedRole);

	function handleCreate() {
		if (!canCreate) return;
		onCreate(selectedRole);
	}
</script>

<Dialog.Root {open} onOpenChange={onOpenChange}>
<Dialog.Content class="sm:max-w-md">
	<Dialog.Header>
		<Dialog.Title>New Agent</Dialog.Title>
		<Dialog.Description>Choose a role for the new agent.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-3">
		<div class="relative">
			<SearchIcon class="absolute left-2.5 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground/40 pointer-events-none" />
			<Input
				bind:value={searchQuery}
				placeholder="Search roles..."
				class="pl-8 h-8 text-xs"
			/>
		</div>
		<div class="flex flex-col gap-1 max-h-48 overflow-y-auto -mx-1 px-1">
			{#each filteredRoles as r}
				<button
					class="w-full flex items-start gap-2.5 px-2.5 py-2 rounded-lg text-left text-xs transition-all {selectedRole === r.name ? 'bg-accent text-accent-foreground ring-1 ring-border' : 'hover:bg-muted/50'}"
					onclick={() => { selectedRole = r.name; }}
				>
					<div class="size-1.5 rounded-full bg-primary/40 mt-1 shrink-0"></div>
					<div class="min-w-0 flex-1">
						<div class="font-medium">{r.name}</div>
						<div class="text-[11px] text-muted-foreground/60 mt-0.5 line-clamp-2">{r.definition}</div>
					</div>
				</button>
			{:else}
				<p class="text-xs text-muted-foreground/50 text-center py-6">No roles match your search.</p>
			{/each}
		</div>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => onOpenChange(false)}>Cancel</Button>
		<Button onclick={handleCreate} disabled={!canCreate}><Bot class="size-3.5" /> Create Agent</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
