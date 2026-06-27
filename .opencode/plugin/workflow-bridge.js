import { execSync } from "node:child_process"
import { existsSync, readFileSync, mkdirSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { join, dirname } from "node:path"
import { randomUUID } from "node:crypto"

const WORKFLOW_DIR = join(homedir(), ".workflow")
const STATE_FILE = join(WORKFLOW_DIR, "state.json")
const WORKFLOW_CMD = "cargo run --release -- --cli"

const PROJECT_DIR = process.cwd()

function readState() {
  if (!existsSync(STATE_FILE)) return {}
  try {
    return JSON.parse(readFileSync(STATE_FILE, "utf8"))
  } catch { return {} }
}

function formatAgentSummary(agent) {
  const status = agent.status || "unknown"
  const id = (agent.id || agent.agent_id || "???").toString().slice(0, 8)
  return `  • \`${agent.name || "unnamed"}\` [${id}] — status: \`${status}\`, depth: ${agent.depth ?? "?"}, role: \`${agent.role || "?"}\``
}

export default (async ({ client, project, directory, $ }) => {
  return {
    config(cfg) {
      cfg.command ??= {}
      cfg.command["workflow-spawn"] = {
        description: "Spawn a workflow agent: /workflow-spawn <task> [role] [values]",
        prompt: JSON.stringify({
          name: "workflow-spawn",
          params: {
            task: { type: "string", description: "Task description for the agent" },
            role: { type: "string", description: "Role for the agent (default: developer)" },
            values: { type: "string", description: "Value statement (optional)" },
          },
        }),
      }
      cfg.command["workflow-status"] = {
        description: "Show workflow runtime status (budget, permits, agents, tasks)",
        prompt: JSON.stringify({ name: "workflow-status" }),
      }
      cfg.command["workflow-list"] = {
        description: "List all active workflow agents and their task graph",
        prompt: JSON.stringify({ name: "workflow-list" }),
      }
    },

    async "command.execute.before"(input, output) {
      const name = input?.command
      if (!name || !name.startsWith("workflow-")) return

      switch (name) {
        case "workflow-status": {
          const state = readState()
          const lines = []
          if (state.budget_total !== undefined) {
            const used = state.budget_used ?? 0
            const total = state.budget_total
            const pct = total > 0 ? Math.round((used / total) * 100) : 0
            lines.push(`**Budget:** ${used}/${total} (${pct}% used)`)
          }
          if (state.permits_available !== undefined) {
            lines.push(`**Permits:** ${state.permits_available}/${state.permits_total ?? "?"} available`)
          }
          lines.push(`**Workflow dir:** \`${WORKFLOW_DIR}\``)
          const checkpointDir = join(WORKFLOW_DIR)
          const hasPool = existsSync(join(checkpointDir, "agent_pool.bin"))
          const hasGraph = existsSync(join(checkpointDir, "task_graph.bin"))
          lines.push(`**Checkpoints:** pool=${hasPool ? "✓" : "✗"}, graph=${hasGraph ? "✓" : "✗"}`)
          output.args = { command: `echo '${lines.join("\\n")}'` }
          break
        }

        case "workflow-list": {
          const outputLines = ["**Workflow Agents:**"]
          for (const file of ["agent_pool.bin"]) {
            const path = join(WORKFLOW_DIR, file)
            if (existsSync(path)) {
              outputLines.push(`  \`${file}\`: ${(readFileSync(path).length / 1024).toFixed(1)} KB`)
            }
          }
          outputLines.push("\nTo get detailed agent info, start the TUI with:\n`cargo run --release`")
          output.args = { command: `echo '${outputLines.join("\\n")}'` }
          break
        }

        case "workflow-spawn": {
          const task = input?.params?.task || "Implement a REST API"
          const role = input?.params?.role || "Senior Rust developer"
          const values = input?.params?.values || "Write maintainable, well-tested code"
          const workdir = PROJECT_DIR
          output.args = {
            command: [
              `echo 'Spawning workflow agent...'`,
              `echo '  Task: ${task}'`,
              `echo '  Role: ${role}'`,
              `cd ${workdir} && WORKFLOW_SPAWN_TASK="${task}" WORKFLOW_SPAWN_ROLE="${role}" WORKFLOW_SPAWN_VALUES="${values}" ${WORKFLOW_CMD}`,
            ].join(" && "),
          }
          break
        }
      }
    },

    async event(input) {
      const event = input
      if (!event?.type) return

      if (event.type === "session.idle" || event.type === "session.status") {
        const sessionId = event?.properties?.sessionID || event?.properties?.info?.sessionID
        if (sessionId && existsSync(STATE_FILE)) {
          const state = readState()
          if (state.selected_models?.length) {
            try { await client.app.log?.({ body: { service: "workflow-bridge", level: "info", message: `Session ${sessionId.slice(0, 8)} — ${state.selected_models.length} model(s) configured, ${state.configured_providers?.length || 0} provider(s)` } }) } catch {}
          }
        }
      }
    },

    tool: {
      workflow_status: {
        name: "workflow_status",
        description: "Check the status of the workflow agent runtime (budget, permits, experience pool, checkpoint state)",
        parameters: { type: "object", properties: {}, required: [] },
        execute: async () => {
          const state = readState()
          const lines = ["# Workflow Runtime Status"]
          if (state.budget_total !== undefined) {
            const used = state.budget_used ?? 0
            const total = state.budget_total
            lines.push(`- Budget: ${used}/${total} (${total > 0 ? Math.round((used / total) * 100) : 0}% used)`)
          }
          if (state.permits_available !== undefined) {
            lines.push(`- Permits: ${state.permits_available}/${state.permits_total ?? "?"}`)
          }
          lines.push(`- State file: ${STATE_FILE}`)
          const poolPath = join(WORKFLOW_DIR, "agent_pool.bin")
          const graphPath = join(WORKFLOW_DIR, "task_graph.bin")
          lines.push(`- Agent pool checkpoint: ${existsSync(poolPath) ? "✓ (" + (readFileSync(poolPath).length / 1024).toFixed(1) + " KB)" : "✗"}`)
          lines.push(`- Task graph checkpoint: ${existsSync(graphPath) ? "✓ (" + (readFileSync(graphPath).length / 1024).toFixed(1) + " KB)" : "✗"}`)
          return { result: lines.join("\n") }
        },
      },
    },
  }
})
