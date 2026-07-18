<script lang="ts">
    import { Bot, User } from "@lucide/svelte";

    let {
        text,
        html,
        role,
        streaming,
    }: {
        text: string;
        html: string;
        role: "user" | "assistant";
        streaming?: boolean;
    } = $props();
</script>

<div class="flex items-start gap-3">
    {#if role === "user"}
        <div
            class="size-7 rounded-full bg-primary/10 flex items-center justify-center shrink-0 mt-0.5"
        >
            <User class="size-3.5 text-primary/70" />
        </div>
    {/if}
    <div class="min-w-0 flex-1">
        <div class="text-xs font-medium text-muted-foreground/60 mb-0.5">
            {role === "user" ? "You" : ""}
        </div>
        {#if role === "user"}
            <div
                class="text-sm leading-relaxed text-foreground/90 whitespace-pre-wrap break-words"
            >
                {text}
            </div>
        {:else}
            <div
                class="text-sm leading-relaxed prose-sm dark:prose-invert max-w-none
				[&_code]:rounded [&_code]:bg-muted-foreground/10 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs [&_code]:font-mono
				[&_pre]:relative [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border/40 [&_pre]:bg-neutral-100 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre]:overflow-x-auto
				[&_pre_code]:bg-transparent [&_pre_code]:p-0
				[&_pre_>_code]:block
				.dark [&_pre]:bg-neutral-900 [&_pre]:border-border/60
				[&_pre_>_code[class*=language-]]:before:content-[attr(class)] [&_pre_>_code[class*=language-]]:before:text-[10px] [&_pre_>_code[class*=language-]]:before:font-medium [&_pre_>_code[class*=language-]]:before:text-muted-foreground/50 [&_pre_>_code[class*=language-]]:before:tracking-wider [&_pre_>_code[class*=language-]]:before:uppercase [&_pre_>_code[class*=language-]]:before:block [&_pre_>_code[class*=language-]]:before:mb-2 [&_pre_>_code[class*=language-]]:before:border-b [&_pre_>_code[class*=language-]]:before:border-border/20 [&_pre_>_code[class*=language-]]:before:pb-1.5
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
                {@html html}{#if streaming}<span
                        class="inline-block w-[2px] h-[1.1em] bg-foreground animate-pulse ml-0.5 align-text-bottom"
                    ></span>{/if}
            </div>
        {/if}
    </div>
</div>
