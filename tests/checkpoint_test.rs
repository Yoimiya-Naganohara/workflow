//! Integration tests for checkpoint save/load roundtrip.
use workflow::checkpoint::Checkpoint;
use workflow::runtime::task_graph::TaskGraph;

/// Helper: create a minimal agent for testing.
fn make_agent(name: &str, role: &str, goal: &str) -> workflow::agent::Agent {
    workflow::agent::Agent {
        id: rand::random(),
        name: name.into(),
        role: role.into(),
        role_template_id: None,
        parent_id: None,
        children: Vec::new(),
        depth: 0,
        goal: goal.into(),
        config: workflow::agent::AgentConfig::default(),
        status: workflow::agent::AgentStatus::Idle,
        result: None,
        child_results: Vec::new(),
        context: Vec::new(),
        last_active_at: 1000,
        tokens_input: 0,
        tokens_output: 0,
        tool_trace: std::collections::VecDeque::new(),
        inbox: std::collections::VecDeque::new(),
        task_id: None,
        sandbox: None,
        retry_count: 0,
        reasoning: String::new(),
    }
}

#[test]
fn test_checkpoint_save_load_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let cp = Checkpoint::with_dir(dir.path().to_path_buf());

    let mut pool = workflow::agent::AgentPool::new();
    let agent = make_agent("integration-agent", "tester", "integration test goal");
    let agent_id = agent.id;
    pool.add_agent(agent);

    let mut graph = TaskGraph::new();
    let root_id = graph.spawn_root("integration goal");
    graph.mark_decomposed(root_id).unwrap();
    let child_id = graph.spawn_child(root_id, "child task").unwrap();
    graph.mark_ready(child_id).unwrap();

    // Save
    cp.save_snapshot(&pool, &graph).unwrap();

    // Load
    let snapshot = cp.restore_snapshot().unwrap().unwrap();
    let loaded_pool = Checkpoint::rehydrate_pool(&snapshot);
    let loaded_graph = snapshot.task_graph;

    // Verify pool
    let loaded_agent = loaded_pool.get_agent(&agent_id).unwrap();
    assert_eq!(loaded_agent.goal, "integration test goal");
    assert_eq!(loaded_agent.role, "tester");
    assert_eq!(loaded_agent.status, workflow::agent::AgentStatus::Idle);

    // Verify graph
    assert!(loaded_graph.contains(&root_id));
    let root = loaded_graph.get(&root_id).unwrap();
    assert_eq!(root.goal, "integration goal");

    let child = loaded_graph.get(&child_id).unwrap();
    assert_eq!(child.parent, Some(root_id));
}

#[test]
fn test_checkpoint_nonexistent_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let cp = Checkpoint::with_dir(dir.path().join("nonexistent_subdir"));

    let result = cp.restore_snapshot().unwrap();
    assert!(result.is_none(), "no checkpoint dir → None");
}

#[test]
fn test_checkpoint_multiple_agents_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let cp = Checkpoint::with_dir(dir.path().to_path_buf());

    let mut pool = workflow::agent::AgentPool::new();
    let ids: Vec<_> = (0..5)
        .map(|i| {
            let mut agent = make_agent(
                &format!("agent-{}", i),
                "worker",
                &format!("goal {}", i),
            );
            agent.depth = i as u32;
            agent.last_active_at = i as u64;
            let id = agent.id;
            pool.add_agent(agent);
            id
        })
        .collect();

    let graph = TaskGraph::new();
    cp.save_snapshot(&pool, &graph).unwrap();

    let snapshot = cp.restore_snapshot().unwrap().unwrap();
    let loaded = Checkpoint::rehydrate_pool(&snapshot);

    assert_eq!(loaded.agents().len(), 5);
    for (i, id) in ids.iter().enumerate() {
        let agent = loaded.get_agent(id).unwrap();
        assert_eq!(agent.name, format!("agent-{}", i));
        assert_eq!(agent.depth, i as u32);
    }
}

#[test]
fn test_checkpoint_clear() {
    let dir = tempfile::TempDir::new().unwrap();
    let cp = Checkpoint::with_dir(dir.path().to_path_buf());

    let pool = workflow::agent::AgentPool::new();
    let graph = TaskGraph::new();

    // Save then clear
    cp.save_snapshot(&pool, &graph).unwrap();
    assert!(cp.exists());

    cp.clear().unwrap();
    assert!(!cp.exists());
    assert!(cp.restore_snapshot().unwrap().is_none());
}
