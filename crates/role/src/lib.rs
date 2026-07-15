use std::collections::{HashMap, VecDeque};

use rig::{agent::Agent, completion::CompletionModel};
use workflow_tool::ToolId;

pub type RoleId = String;

/// Maximum number of experiences retained per role (FIFO eviction).
const MAX_EXPERIENCES_PER_ROLE: usize = 1024; //TODO: MAKE THIS CONFIGURABLE

// ── Role ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Role {
    name: String,
    definition: String,
    tools: Vec<ToolId>,
}

impl Role {
    pub fn new(name: String, definition: String, tools: Vec<ToolId>) -> Self {
        Self {
            name,
            definition,
            tools,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn definition(&self) -> &str {
        &self.definition
    }

    pub fn tools(&self) -> &[u32] {
        &self.tools
    }
}
#[derive(Clone)]
pub struct RolePool {
    roles: HashMap<RoleId, Role>,
}

impl Default for RolePool {
    fn default() -> Self {
        Self {
            roles: HashMap::from([
                (
                    RoleId::default(),
                    Role::new(
                        "planner".to_string(),
                        "I should help user to plan".to_string(),
                        vec![ToolId::default()],
                    ),
                ),
                (
                    RoleId::from("executor"),
                    Role::new(
                        "executor".to_string(),
                        "I should execute the plan".to_string(),
                        vec![],
                    ),
                ),
            ]),
        }
    }
}

impl RolePool {
    pub fn new(roles: HashMap<RoleId, Role>) -> Self {
        Self { roles }
    }

    pub fn get(&self, role_id: &RoleId) -> Option<&Role> {
        self.roles.get(role_id)
    }

    pub fn add(&mut self, id: RoleId, role: Role) {
        self.roles.insert(id, role);
    }

    pub fn list(&self) -> Vec<&Role> {
        self.roles.values().collect()
    }
}

// ── Experience ──────────────────────────────────────────────
// TODO: MAKE THESE CONFIGURABLE
const GOAL_MAX_LENGTH: usize = 1024;
const TOOL_CALLS_MAX_LENGTH: usize = 1024;
const SUMMARY_MAX_LENGTH: usize = 1024;
const COMMENT_MAX_LENGTH: usize = 1024;

pub struct Experience {
    role_id: RoleId,
    goal: String,
    tool_calls: Vec<String>,
    summary: String,
    comment: String,
    timestamp: u64,
}
impl Experience {
    pub fn new(
        role_id: RoleId,
        goal: String,
        tool_calls: Vec<String>,
        summary: String,
        comment: String,
        timestamp: u64,
    ) -> Option<Self> {
        if goal.len() > GOAL_MAX_LENGTH
            || tool_calls.len() > TOOL_CALLS_MAX_LENGTH
            || summary.len() > SUMMARY_MAX_LENGTH
            || comment.len() > COMMENT_MAX_LENGTH
        {
            return None;
        }
        Self {
            role_id,
            goal,
            tool_calls,
            summary,
            comment,
            timestamp,
        }
        .into()
    }

    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub fn comment(&self) -> &str {
        &self.comment
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
}
// ── ExperiencePool (per-role ring buffer) ───────────────────

/// Per-role FIFO ring buffer of [`Experience`] entries.
///
/// Each role gets its own queue capped at [`MAX_EXPERIENCES_PER_ROLE`].
/// When the limit is reached, the oldest entry is evicted before
/// appending the new one.
pub struct ExperiencePool {
    experiences: HashMap<RoleId, VecDeque<Experience>>,
}

impl Default for ExperiencePool {
    fn default() -> Self {
        Self::new()
    }
}

impl ExperiencePool {
    pub fn new() -> Self {
        Self {
            experiences: HashMap::new(),
        }
    }

    /// Return all experiences for a role (oldest first).
    ///
    /// Returns an empty slice when the role has no entries or when the
    /// internal buffer is non-contiguous (use [`query_vec`] for that case).
    pub fn query(&self, id: &RoleId) -> &[Experience] {
        self.experiences
            .get(id)
            .and_then(|deque| {
                let (a, b) = deque.as_slices();
                if b.is_empty() { Some(a) } else { None }
            })
            .unwrap_or(&[])
    }

    /// Return all experiences for a role as a collected Vec (handles
    /// non-contiguous internal storage).
    pub fn query_vec(&self, id: &RoleId) -> Vec<&Experience> {
        self.experiences
            .get(id)
            .map(|deque| deque.iter().collect())
            .unwrap_or_default()
    }

    /// Ring-buffer append.  Evicts oldest when at capacity.
    pub fn add(&mut self, id: RoleId, experience: Experience) {
        let entry = self.experiences.entry(id).or_default();
        if entry.len() >= MAX_EXPERIENCES_PER_ROLE {
            entry.pop_front();
        }
        entry.push_back(experience);
    }

    /// Number of roles with at least one experience.
    pub fn len(&self) -> usize {
        self.experiences.len()
    }

    pub fn is_empty(&self) -> bool {
        self.experiences.is_empty()
    }

    /// Remove all experiences for a role.
    pub fn clear_role(&mut self, id: &RoleId) {
        self.experiences.remove(id);
    }

    /// Remove all experiences across all roles.
    pub fn clear(&mut self) {
        self.experiences.clear();
    }
}

// ── Optimizer ───────────────────────────────────────────────
pub trait Embed {
    fn embed(&self, str: &str) -> Vec<f32>;
}
pub struct Optimizer {
    embedder: Box<dyn Embed>,
}

impl Optimizer {
    pub fn new(embedder: Box<dyn Embed>) -> Self {
        Self { embedder }
    }
    fn cosine(&self, a: &[f32], b: &[f32]) -> f32 {
        let dot_product = a.iter().zip(b.iter()).map(|(a, b)| a * b).sum::<f32>();
        let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot_product / (norm_a * norm_b)
    }

    pub async fn optimize(
        &self,
        role: &Role,
        agent: &Agent<impl CompletionModel>,
        experiences: &[Experience],
    ) -> Role {
        let mut prompt = String::from(&role.definition);
        for experience in experiences {
            let goal_len = experience.goal.len();
            let goal = &experience.goal;
            let goal_vec = self.embedder.embed(goal);

            let (
                mut peak_s,
                mut bottom_s,
                mut peak_summary,
                mut peak_summary_vec,
                mut peak_t,
                mut bottom_t,
                mut peak_tool_call,
            ) = (
                f32::NEG_INFINITY,
                f32::INFINITY,
                "".to_string(),
                Vec::new(),
                f32::NEG_INFINITY,
                f32::INFINITY,
                &"".to_string(),
            );
            for summary in experience
                .summary
                .chars()
                .collect::<Vec<char>>()
                .chunks(goal_len)
            {
                let summary = summary.iter().collect::<String>();
                let summary_vec = self.embedder.embed(&summary);
                let similarity = self.cosine(&goal_vec, &summary_vec);

                peak_s = peak_s.max(similarity);
                bottom_s = bottom_s.min(similarity);
                if similarity == peak_s {
                    peak_summary = summary;
                    peak_summary_vec = summary_vec;
                }
            }
            for tool in &experience.tool_calls {
                let tool_vec = self.embedder.embed(tool);
                let similarity = self.cosine(&peak_summary_vec, &tool_vec);
                peak_t = peak_t.max(similarity);
                bottom_t = bottom_t.min(similarity);
                if similarity == peak_t {
                    peak_tool_call = tool;
                }
            }
            prompt.push_str(
                format!(
                    "\ngoal: {goal}\nsummary similarity [{peak_s}, {bottom_s}]: {peak_summary}\ntool similarity [{peak_t}, {bottom_t}]: {peak_tool_call}\n"
                ).as_str()
            );
        }
        if let Ok(prompt_response) = agent.runner(prompt).run().await {
            Role::new(
                role.name.clone(),
                prompt_response.output,
                role.tools.clone(),
            )
        } else {
            role.clone()
        }
    }
}
