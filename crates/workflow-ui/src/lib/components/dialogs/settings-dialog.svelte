<script lang="ts">
	import CheckIcon from "@lucide/svelte/icons/check";
	import ChevronsUpDownIcon from "@lucide/svelte/icons/chevrons-up-down";
	import { tick } from "svelte";
	import * as Command from "$lib/components/ui/command";
	import * as Dialog from "$lib/components/ui/dialog";
	import * as Popover from "$lib/components/ui/popover";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { cn } from "$lib/utils";
	import type { ProviderEntry, ProviderModel } from "$lib/types";

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
		onRefreshProviders: () => void;
	} = $props();

	let localProvider = $state("");
	let localModel = $state("");
	let localApiKey = $state("");

	const loading = $derived(providers.length === 0 && refreshing);

	$effect(() => {
		if (open) {
			localProvider = selectedProvider;
			localModel = selectedModel;
			localApiKey = apiKey;
		}
	});

	let wasOpen = $state(false);
	$effect(() => {
		if (open && !wasOpen) onRefreshProviders();
		wasOpen = open;
	});

	const currentProvider = $derived(providers.find(p => p.id === localProvider));
	const availableModels = $derived(currentProvider?.models ?? []);
	const needsApiKey = $derived(!!currentProvider?.api_url);

	let providerOpen = $state(false);
	let modelOpen = $state(false);
	let providerTriggerRef = $state<HTMLButtonElement>(null!);
	let modelTriggerRef = $state<HTMLButtonElement>(null!);

	const selectedProviderName = $derived(providers.find(p => p.id === localProvider)?.name);
	const selectedModelName = $derived(currentProvider?.models.find((m: ProviderModel) => m.id === localModel)?.name);

	function closeAndFocus(ref: HTMLButtonElement) {
		tick().then(() => ref.focus());
	}

	function handleOpenChange(o: boolean) {
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
	<div class="space-y-4">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-2">
				<span class="text-xs font-medium text-muted-foreground">Providers</span>
				{#if refreshing}
					<span class="size-3 rounded-full border-2 border-muted-foreground/30 border-t-foreground animate-spin"></span>
				{/if}
			</div>
			<Button variant="ghost" size="icon-xs" disabled={refreshing} onclick={onRefreshProviders}>
				<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3">
					<path d="M21 2v6h-6" /><path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
					<path d="M3 22v-6h6" /><path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
				</svg>
			</Button>
		</div>

		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="provider">Provider</label>
			<Popover.Root bind:open={providerOpen}>
				<Popover.Trigger bind:ref={providerTriggerRef} id="provider" disabled={loading}>
					{#snippet child({ props }: { props: Record<string, unknown> })}
						<Button
							{...props}
							variant="outline"
							class="w-full justify-between text-sm h-9"
							role="combobox"
							aria-expanded={providerOpen}
							disabled={loading}
						>
							{selectedProviderName || (loading ? "Loading providers..." : providers.length === 0 ? "No providers available" : "Select a provider")}
							<ChevronsUpDownIcon class="size-4 opacity-50 shrink-0" />
						</Button>
					{/snippet}
				</Popover.Trigger>
				<Popover.Content class="p-0" style={providerTriggerRef ? `width: ${providerTriggerRef.clientWidth}px` : undefined}>
					<Command.Root>
						<Command.Input placeholder="Search provider..." />
						<Command.List>
							<Command.Empty>No provider found.</Command.Empty>
							<Command.Group>
								{#each providers as p (p.id)}
									<Command.Item
										value={p.id}
										onSelect={() => {
											localProvider = p.id;
											localModel = "";
											providerOpen = false;
											closeAndFocus(providerTriggerRef);
										}}
									>
										<CheckIcon class={cn("me-2 size-4", localProvider !== p.id && "text-transparent")} />
										{p.name}
									</Command.Item>
								{/each}
							</Command.Group>
						</Command.List>
					</Command.Root>
				</Popover.Content>
			</Popover.Root>
		</div>

		{#if currentProvider}
			<div class="flex flex-col gap-1.5">
				<label class="text-xs font-medium text-muted-foreground" for="model">Model</label>
				<Popover.Root bind:open={modelOpen}>
				<Popover.Trigger bind:ref={modelTriggerRef} id="model" disabled={availableModels.length === 0}>
					{#snippet child({ props }: { props: Record<string, unknown> })}
						<Button
							{...props}
							variant="outline"
							class="w-full justify-between text-sm h-9"
							role="combobox"
							aria-expanded={modelOpen}
							disabled={availableModels.length === 0}
						>
							{selectedModelName || (availableModels.length === 0 ? "No models available" : "Select a model")}
							<ChevronsUpDownIcon class="size-4 opacity-50 shrink-0" />
						</Button>
					{/snippet}
				</Popover.Trigger>
					<Popover.Content class="p-0" style={modelTriggerRef ? `width: ${modelTriggerRef.clientWidth}px` : undefined}>
						<Command.Root>
							<Command.Input placeholder="Search model..." />
							<Command.List>
								<Command.Empty>No model found.</Command.Empty>
								<Command.Group>
									{#each availableModels as m (m.id)}
										<Command.Item
											value={m.id}
											onSelect={() => {
												localModel = m.id;
												modelOpen = false;
												closeAndFocus(modelTriggerRef);
											}}
										>
											<CheckIcon class={cn("me-2 size-4", localModel !== m.id && "text-transparent")} />
											{m.name}{m.supports_tools ? " (tools)" : ""}
										</Command.Item>
									{/each}
								</Command.Group>
							</Command.List>
						</Command.Root>
					</Popover.Content>
				</Popover.Root>
			</div>

			{#if needsApiKey}
				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-medium text-muted-foreground" for="api-key">API Key</label>
					<Input id="api-key" type="password" bind:value={localApiKey} placeholder="sk-..." />
					<p class="text-[10px] text-muted-foreground/50">
						Base URL: {currentProvider.api_url}
					</p>
				</div>
			{/if}
		{/if}

		{#if configured}
			<div class="flex items-center gap-1.5 rounded-lg bg-emerald-500/5 border border-emerald-500/20 px-3 py-2">
				<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 text-emerald-500 shrink-0">
					<path d="M20 6 9 17l-5-5" />
				</svg>
				<p class="text-xs text-emerald-600 dark:text-emerald-400">
					Configured: {selectedProvider} / {selectedModel}
				</p>
			</div>
		{/if}

		{#if !refreshing && providers.length === 0 && !loading}
			<div class="flex items-center gap-1.5 rounded-lg bg-muted/50 px-3 py-2">
				<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 text-muted-foreground/50 shrink-0">
					<line x1="1" y1="1" x2="23" y2="23" /><path d="M16.72 3.7A10 10 0 0 0 3.7 16.72" />
					<path d="M7.28 20.3A10 10 0 0 0 20.3 7.28" />
				</svg>
				<p class="text-xs text-muted-foreground/60">No providers found. Click refresh to fetch from network.</p>
			</div>
		{/if}
	</div>
	<Dialog.Footer>
		<Button variant="ghost" onclick={() => onOpenChange(false)}>Cancel</Button>
		<Button
			disabled={!localProvider || !localModel || (needsApiKey && !localApiKey)}
			onclick={handleSave}
		>Save</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>
