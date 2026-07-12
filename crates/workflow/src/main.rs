use workflow_core::Runtime;

#[tokio::main]
async fn main() {
    let runtime = Runtime::new();
    runtime.run().await;
}
