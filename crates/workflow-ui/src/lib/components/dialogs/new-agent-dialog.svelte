<script lang="ts">
	import * as Dialog from "$lib/components/ui/dialog";
	import * as Select from "$lib/components/ui/select";
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import { Bot } from "@lucide/svelte";
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
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen) {
			selectedRole = roles[0]?.name ?? "";
		}
		wasOpen = open;
	});

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
<Dialog.Content>
	<Dialog.Header>
		<Dialog.Title>New Agent</Dialog.Title>
		<Dialog.Description>Choose a role for the new agent.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-3">
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="new-role">Role</label>
			<Select.Root bind:value={selectedRole} type="single">
				<Select.Trigger class="w-full text-xs h-8" id="new-role">
					{selectedRole || "Select a role..."}
				</Select.Trigger>
				<Select.Content>
					{#each roles as r}
						<Select.Item value={r.name}>{r.name}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
		<Card class="p-3 bg-muted/50">
			<p class="text-xs text-muted-foreground leading-relaxed">{roleDesc}</p>
		</Card>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => onOpenChange(false)}>Cancel</Button>
		<Button onclick={handleCreate} disabled={!canCreate}><Bot class="size-3.5" /> Create Agent</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
