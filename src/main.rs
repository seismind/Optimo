use anyhow::Result;
use std::path::PathBuf;

mod ocrys;
mod state;
mod state_bridge;
mod task;

#[tokio::main]
async fn main() -> Result<()> {
    // Bootstrap the application state (paths, runtime dirs).
    let state = crate::state::AppState::new().await?;

    // CLI minimal: pass file paths as args.
    let docs: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if docs.is_empty() {
        eprintln!("Usage: optimo <file1> <file2> ...");
        return Ok(());
    }

    // Orchestrate: delegate CPU-bound work to the worker boundary.
    task::process_documents(&state, docs).await?;

    Ok(())
}
