<script lang="ts">
	import { highlightCode, isReady } from './highlighter';
	import { Copy, Check } from '@lucide/svelte';

	let {
		lang = 'text',
		text,
	}: {
		lang?: string;
		text: string;
	} = $props();

	let copied = $state(false);
	let settledText = $state('');
	let settled = $state(false);

	// Self-contained raf-debounce: highlights one frame after text stops
	// changing. During active streaming, each chunk cancels the previous
	// frame so highlighting only lands when text is stable. Needs no
	// external streaming signal — works for both streaming and static.
	$effect(() => {
		const t = text;
		settled = false;
		settledText = '';
		const raf = requestAnimationFrame(() => {
			settledText = t;
			settled = true;
		});
		return () => cancelAnimationFrame(raf);
	});

	let highlighted = $derived(
		settled && isReady() ? highlightCode(settledText, lang) : '',
	);

	async function copy() {
		try {
			await navigator.clipboard.writeText(text);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			/* clipboard unavailable */
		}
	}
</script>

<div class="group/pre relative my-2 rounded-lg border border-border/40 overflow-hidden">
	{#if highlighted}
		{@html highlighted}
	{:else}
		<pre class="p-3 text-xs leading-relaxed overflow-x-auto bg-muted/30"><code>{text}</code></pre>
	{/if}
	<button
		onclick={copy}
		class="absolute top-2 right-2 z-10 size-7 rounded-md flex items-center justify-center text-muted-foreground/50 hover:text-foreground hover:bg-background/80 opacity-0 group-hover/pre:opacity-100 transition-all"
		title={copied ? 'Copied!' : 'Copy code'}
	>
		{#if copied}
			<Check class="size-3.5" />
		{:else}
			<Copy class="size-3.5" />
		{/if}
	</button>
</div>
