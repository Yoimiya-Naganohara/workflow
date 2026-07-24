<script lang="ts">
    import SvelteMarkdown from "@humanspeak/svelte-markdown";
    import MarkdownCode from "$lib/markdown/markdown-code.svelte";

    let {
        text,
        role,
        streaming,
    }: {
        text: string;
        role: "user" | "assistant";
        streaming?: boolean;
    } = $props();
</script>

{#if role === "user"}
    <div class="flex justify-end">
        <div
            class="inline-block max-w-[75%] bg-primary/10 rounded-2xl rounded-br-sm px-4 py-2.5 text-sm leading-relaxed prose-sm dark:prose-invert
"
        >
            <SvelteMarkdown source={text}>
                {#snippet code({ lang, text: codeText })}
                    <MarkdownCode lang={lang || "text"} text={codeText} />
                {/snippet}
            </SvelteMarkdown>
        </div>
    </div>
{:else}
    <SvelteMarkdown source={text} {streaming}>
        {#snippet code({ lang, text: codeText })}
            <MarkdownCode lang={lang || "text"} text={codeText} />
        {/snippet}
    </SvelteMarkdown>
{/if}
