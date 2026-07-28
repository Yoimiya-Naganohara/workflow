use std::{
    io::{Write, stdin, stdout},
    sync::Arc,
};

use anyhow::Context;
use workflow_config::ProjectConfig;
use workflow_core::Runtime;

fn main() -> anyhow::Result<()> {
    let project = parse_project_arg();
    tokio::runtime::Runtime::new()
        .context("failed to create tokio runtime")?
        .block_on(async_main(project))
}

fn parse_project_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if (arg == "--project" || arg == "-p")
            && let Some(path) = args.next()
        {
            return Some(path);
        }
        // Support --project=<path> form.
        if let Some(path) = arg.strip_prefix("--project=") {
            return Some(path.to_string());
        }
        if let Some(path) = arg.strip_prefix("-p=") {
            return Some(path.to_string());
        }
    }
    None
}

async fn async_main(project_path: Option<String>) -> anyhow::Result<()> {
    let mut config = workflow_core::RuntimeConfig::resolve();

    // If a project path was provided, set up project config.
    if let Some(path) = project_path {
        let canonical = std::fs::canonicalize(&path)
            .context(format!("Failed to resolve project path: {path}"))?;
        let path_str = canonical.to_string_lossy().to_string();
        let canonical_path = if canonical.is_dir() {
            path_str
        } else {
            // If it's a file, use its parent directory.
            canonical
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(path_str)
        };
        config.project = Some(ProjectConfig::new(canonical_path));
        eprintln!(
            "📂 Opened project: {}",
            config.project.as_ref().unwrap().name()
        );
    }

    let runtime = Arc::new(Runtime::try_new(config)?);
    runtime
        .initialize()
        .await
        .context("failed to initialize runtime")?;

    let root_id = runtime
        .snapshot(None)
        .await
        .selected
        .expect("runtime initialization creates a root agent");
    let mut events = runtime.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let Some(text) = event.text() {
                print!("{text}");
                let _ = stdout().flush();
            } else if let Some(error) = event.error() {
                eprintln!("agent error: {error}");
            }
        }
    });

    if runtime.project().is_some() {
        eprintln!(
            "💡 Agents can use `get_project_info` and `read_project_file` tools to explore the project."
        );
    }

    loop {
        let mut input = String::new();
        if stdin().read_line(&mut input)? == 0 {
            break;
        }
        if runtime.send_message(root_id, input).await.is_err() {
            break;
        }
    }

    Ok(())
}
