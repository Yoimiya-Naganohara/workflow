<script lang="ts">
	import * as Dialog from "$lib/components/ui/dialog";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import type { ProviderEntry } from "$lib/types";

	let {
		open,
		providers,
		selectedProvider,
		selectedModel,
		apiKey,
		configured,
		refreshing,
		onOpenChange,
		onConfigure,
		onLoadProviders,
		onRefreshProviders,
	}: {
		open: boolean;
		providers: ProviderEntry[];
		selectedProvider: string;
		selectedModel: string;
		apiKey: string;
		configured: boolean;
		refreshing: boolean;
		onOpenChange: (open: boolean) => void;
		onConfigure: (providerId: string, apiKey: string, model: string) => void;
		onLoadProviders: () => void;
		onRefreshProviders: () => void;
	} = $props();

	let localProvider = $state(selectedProvider);
	let localModel = $state(selectedModel);
	let localApiKey = $state(apiKey);

	const currentProvider = $derived(providers.find(p => p.id === localProvider));
	const availableModels = $derived(currentProvider?.models ?? []);

	function handleOpenChange(o: boolean) {
		if (o) onLoadProviders();
		onOpenChange(o);
	}

	function handleSave() {
		onConfigure(localProvider, localApiKey, localModel);
	}
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
<Dialog.Content>
	<Dialog.Header>
		<Dialog.Title>Settings</Dialog.Title>
		<Dialog.Description>Select an LLM provider and configure your API key.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-3">
		<Button
			variant="outline"
			size="sm"
			class="w-full text-xs"
			disabled={refreshing}
			onclick={onRefreshProviders}
		>
			{refreshing ? "Refreshing..." : "Refresh provider list from network"}
		</Button>
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="provider">Provider</label>
			<select
				id="provider"
				bind:value={localProvider}
				onchange={() => { localModel = ""; }}
				class="flex h-9 w-full rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm transition-colors focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-3 outline-none"
			>
				<option value="" disabled>Select a provider</option>
				{#each providers as p}
					<option value={p.id}>{p.name}</option>
				{/each}
			</select>
		</div>

		{#if currentProvider}
			<div class="flex flex-col gap-1.5">
				<label class="text-xs font-medium text-muted-foreground" for="model">Model</label>
				<select
					id="model"
					bind:value={localModel}
					class="flex h-9 w-full rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm transition-colors focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-3 outline-none"
				>
					<option value="" disabled>Select a model</option>
					{#each availableModels as m}
						<option value={m.id}>{m.name}</option>
					{/each}
				</select>
			</div>

			<div class="flex flex-col gap-1.5">
				<label class="text-xs font-medium text-muted-foreground" for="api-key">API Key</label>
				<Input id="api-key" type="password" bind:value={localApiKey} placeholder={currentProvider.api_url ? `sk-...` : "No API key needed"} />
			</div>
		{/if}

		{#if configured}
			<p class="text-xs text-muted-foreground">
				Currently using: {selectedProvider} / {selectedModel}
			</p>
		{/if}
	</div>
	<Dialog.Footer>
		<Button variant="ghost" onclick={() => onOpenChange(false)}>Cancel</Button>
		<Button
			disabled={!localProvider || !localModel}
			onclick={handleSave}
		>Save</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
