<script lang="ts">
    import { Brain } from "@lucide/svelte";
    import type { AgentInfo, AgentId, AgentStatus } from "$lib/types";
    import type { McpConnectionInfo, McpServerConfig } from "$lib/types";

    import AgentList from "./agent-list.svelte";
    import McpPanel from "./mcp-panel.svelte";
    import ProjectPanel from "./project-panel.svelte";
    import SidebarFooter from "./sidebar-footer.svelte";

    let {
            agents,
            selected,
            statuses,
            onSelect,
            onCreateClick,
            onRemoveAgent,
            roles,
            mcpConfigs,
            mcpConnections,
            mcpExpanded,
            onToggleMcp,
            onAddMcpServer,
            onRemoveMcpServer,
            projectPath,
            projectName,
            projectExpanded,
            onToggleProject,
            onChangeProject,
            onClearProject,
        }: {
            agents: AgentInfo[];
            selected: AgentId | null;
            statuses: Map<AgentId, AgentStatus>;
            onSelect: (id: AgentId) => void;
            onCreateClick: () => void;
            onRemoveAgent: (id: AgentId) => void;
            roles: import("$lib/types").RoleInfo[];
            mcpConfigs: McpServerConfig[];
            mcpConnections: McpConnectionInfo[];
            mcpExpanded: boolean;
            onToggleMcp: () => void;
            onAddMcpServer: (config: McpServerConfig) => void;
            onRemoveMcpServer: (name: string) => void;
            projectPath: string;
            projectName: string;
            projectExpanded: boolean;
            onToggleProject: () => void;
            onChangeProject: (path: string) => void;
            onClearProject: () => void;
        } = $props();
</script>

<aside class="flex flex-col w-60 min-w-60 bg-transparent overflow-hidden shrink-0">
	<AgentList
		{agents}
		{selected}
		{statuses}
		{onSelect}
		{onCreateClick}
		{onRemoveAgent}
	/>

	<McpPanel
		configs={mcpConfigs}
		connections={mcpConnections}
		expanded={mcpExpanded}
		onToggle={onToggleMcp}
		onAdd={onAddMcpServer}
		onRemove={onRemoveMcpServer}
	/>

	<div class="shrink-0 px-3 py-1.5 flex items-center gap-2 text-xs text-muted-foreground/60">
		<Brain class="size-3" />
		<span>{roles.length} role{roles.length !== 1 ? "s" : ""}</span>
	</div>

	<ProjectPanel
		{projectPath}
		{projectName}
		expanded={projectExpanded}
		onToggle={onToggleProject}
		onChangeProject={onChangeProject}
		onClearProject={onClearProject}
	/>

	<SidebarFooter />
</aside>
