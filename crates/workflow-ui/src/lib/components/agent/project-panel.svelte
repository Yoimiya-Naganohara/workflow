<script lang="ts">
    import { Button } from "$lib/components/ui/button";
    import { Input } from "$lib/components/ui/input";
    import { Folder, FolderOpen, ChevronRight, ChevronDown, Plus, X, Loader2, AlertCircle } from "@lucide/svelte";
    import { invoke } from "@tauri-apps/api/core";

    let {
        projectPath,
        projectName,
        expanded,
        onToggle,
        onChangeProject,
        onClearProject,
    }: {
        projectPath: string;
        projectName: string;
        expanded: boolean;
        onToggle: () => void;
        onChangeProject: (path: string) => void;
        onClearProject: () => void;
    } = $props();

    let showAddForm = $state(false);
    let adding = $state(false);
    let newPath = $state("");
    let errorMsg = $state("");

    function resetForm() {
        newPath = "";
        showAddForm = false;
        errorMsg = "";
    }

    async function pickFolder() {
        try {
            const selected = await invoke<string | null>("pick_folder");
            if (selected) {
                newPath = selected;
                errorMsg = "";
            }
        } catch (e) {
            errorMsg = `Failed to open dialog: ${e}`;
        }
    }

    async function handleChange() {
        if (!newPath.trim()) return;
        adding = true;
        errorMsg = "";
        try {
            await onChangeProject(newPath.trim());
            resetForm();
        } catch (e) {
            errorMsg = `${e}`;
        } finally {
            adding = false;
        }
    }
</script>

<div class="shrink-0 border-t border-border/40">
    <button
        class="w-full flex items-center justify-between px-3 py-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider hover:bg-muted/30 transition-colors"
        onclick={onToggle}
    >
        <div class="flex items-center gap-2">
            {#if projectName}
                <FolderOpen class="size-3.5 text-amber-500" />
            {:else}
                <Folder class="size-3.5" />
            {/if}
            <span>Project</span>
        </div>
        <div class="flex items-center gap-1">
            <Button
                variant="ghost"
                size="icon-xs"
                onclick={(e) => {
                    e.stopPropagation();
                    showAddForm = !showAddForm;
                    if (!showAddForm) resetForm();
                }}
                title={projectName ? "Change project" : "Open project"}
                aria-label="Open project"
            >
                <Plus class="size-3" />
            </Button>
            {#if expanded}
                <ChevronDown class="size-3 transition-transform" />
            {:else}
                <ChevronRight class="size-3 transition-transform" />
            {/if}
        </div>
    </button>

    {#if expanded}
        <div class="px-3 pb-2 flex flex-col gap-1 max-h-32 overflow-y-auto no-scrollbar">
            {#if projectName}
                <div class="flex items-start gap-2 px-2 py-1.5 rounded-md text-xs group relative hover:bg-muted/30 transition-colors">
                    <FolderOpen class="size-3.5 text-amber-500 shrink-0 mt-0.5" />
                    <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-1.5">
                            <span class="font-medium truncate">{projectName}</span>
                        </div>
                        <p class="text-[11px] text-muted-foreground/60 mt-0.5 truncate leading-tight" title={projectPath}>
                            {projectPath}
                        </p>
                    </div>
                    <div class="absolute right-0.5 inset-y-0 flex items-center opacity-0 group-hover:opacity-100 transition-opacity">
                        <Button
                            variant="ghost"
                            size="icon-xs"
                            class="text-muted-foreground hover:text-destructive"
                            onclick={(e) => { e.stopPropagation(); onClearProject(); }}
                            title="Close project"
                        >
                            <X class="size-3" />
                        </Button>
                    </div>
                </div>
            {:else}
                <div class="flex flex-col items-center gap-2 py-4 text-center px-2">
                    <div class="size-8 rounded-full bg-muted/50 flex items-center justify-center">
                        <Folder class="size-3.5 text-muted-foreground/40" />
                    </div>
                    <p class="text-xs text-muted-foreground">No project open</p>
                    <Button variant="outline" size="xs" onclick={() => { showAddForm = true; }}>
                        <Plus class="size-3" /> Open Folder
                    </Button>
                </div>
            {/if}

            <!-- Change/add project form -->
            {#if showAddForm}
                <div class="flex flex-col gap-2 px-2 py-2 mt-1 rounded-md bg-muted/30 border border-border/30">
                    <span class="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                        {projectName ? "Change Project" : "Open Project"}
                    </span>

                    <div class="flex flex-col gap-1">
                        <label for="project-path" class="text-[10px] text-muted-foreground/70">Folder path</label>
                        <div class="flex gap-1">
                            <div class="flex-1">
                                <Input
                                    id="project-path"
                                    type="text"
                                    placeholder="/path/to/project"
                                    class="h-6 text-xs px-2"
                                    bind:value={newPath}
                                />
                            </div>
                            <Button variant="outline" size="icon-xs" onclick={pickFolder} title="Browse">
                                <Folder class="size-3" />
                            </Button>
                        </div>
                    </div>

                    {#if errorMsg}
                        <div class="flex items-start gap-1.5 px-2 py-1.5 rounded bg-red-500/10 border border-red-500/20">
                            <AlertCircle class="size-3 text-red-500 shrink-0 mt-0.5" />
                            <p class="text-[10px] text-red-600 dark:text-red-400 leading-tight">{errorMsg}</p>
                        </div>
                    {/if}

                    <div class="flex items-center gap-1.5 mt-1">
                        <Button
                            variant="default"
                            size="xs"
                            class="flex-1 h-6 text-[11px]"
                            disabled={adding || !newPath.trim()}
                            onclick={handleChange}
                        >
                            {#if adding}
                                <Loader2 class="size-2.5 animate-spin" />
                            {/if}
                            {projectName ? "Change" : "Open"}
                        </Button>
                        <Button
                            variant="ghost"
                            size="xs"
                            class="h-6 text-[11px]"
                            onclick={resetForm}
                        >
                            Cancel
                        </Button>
                    </div>
                </div>
            {/if}
        </div>
    {/if}
</div>
