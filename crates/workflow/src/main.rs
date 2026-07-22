use std::{
    io::{Write, stdin, stdout},
    sync::Arc,
};

use anyhow::Context;
use workflow_core::Runtime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = Arc::new(Runtime::new());
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
