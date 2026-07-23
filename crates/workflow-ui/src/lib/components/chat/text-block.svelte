<script lang="ts">
    import { User } from "@lucide/svelte";
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

<div class="flex items-start gap-3 animate-in">
    {#if role === "user"}
        <div
            class="size-7 rounded-full bg-primary/10 flex items-center justify-center shrink-0 mt-0.5 ring-1 ring-primary/20"
        >
            <User class="size-3.5 text-primary/70" />
        </div>
    {/if}
    <div class="min-w-0 flex-1">
        {#if role === "user"}
            <div class="text-xs font-medium text-muted-foreground/50 mb-1">You</div>
        {/if}
        <div
            class="text-sm leading-relaxed prose-sm dark:prose-invert max-w-none
			[&_code]:rounded [&_code]:bg-muted-foreground/10 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs [&_code]:font-mono
			[&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border/40 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre]:overflow-x-auto
			[&_pre_code]:bg-transparent [&_pre_code]:p-0
			[&_pre_>_code]:block
			.dark [&_pre]:border-border/60
			[&_blockquote]:border-l-2 [&_blockquote]:border-muted-foreground/20 [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground/70 [&_blockquote]:italic
			[&_ul]:list-disc [&_ul]:pl-4 [&_ol]:list-decimal [&_ol]:pl-4
			[&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_a]:decoration-primary/30
			[&_h1]:text-base [&_h1]:font-semibold [&_h2]:text-sm [&_h2]:font-semibold [&_h3]:text-sm [&_h3]:font-medium
			[&_p]:my-0 [&_p+_p]:mt-2
			[&_:not(pre)_>_code]:before:content-['`'] [&_:not(pre)_>_code]:after:content-['`']
			[&_hr]:border-border/50
			[&_table]:w-full [&_table]:text-xs [&_th]:text-left [&_th]:font-medium [&_th]:text-muted-foreground [&_th]:pb-1 [&_td]:py-0.5 [&_td]:border-t [&_td]:border-border/30
		"
        >
            <SvelteMarkdown source={text} streaming={true}>
                {#snippet code({ lang, text: codeText })}
                    <MarkdownCode lang={lang || 'text'} text={codeText} />
                {/snippet}
            </SvelteMarkdown>
            {#if streaming}
                <span class="inline-block size-2 rounded-full bg-emerald-500/60 ml-0.5 align-middle" style="animation: pulse-dot 1s ease-in-out infinite"></span>
            {/if}
        </div>
    </div>
</div>
