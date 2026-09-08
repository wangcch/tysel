use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;
use tysel_runtime::DurablePlane;
use tysel_task::TaskId;

#[derive(Subcommand)]
pub enum DurableCommand {
    /// Read a retained completion without executing the application.
    Result { task_id: String },
    /// Delete a bounded batch of explicitly completed tasks and their retained history.
    Prune {
        /// Retain at least this many seconds of completed history.
        #[arg(long, default_value_t = 604_800)]
        older_than_secs: u64,
        /// Maximum tasks in one batch (1..=100); active tasks are never removed.
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
}

pub fn run(command: DurableCommand, project: &crate::project::ProjectContext) -> Result<()> {
    let path = if project.manifest.durable.store == "sqlite" {
        &project.manifest.durable.path
    } else {
        ""
    };
    if let Some(path) = DurablePlane::event_log_path(path, Some(&project.root))
        && !path.is_file()
    {
        bail!("durable event store does not exist");
    }
    let store = DurablePlane::open_store(path, Some(&project.root))?
        .context("durable event store is not configured")?;
    let output = match command {
        DurableCommand::Result { task_id } => {
            let id = TaskId(u128::from_str_radix(&task_id, 16).context("invalid durable task ID")?);
            let completion = store.completion(id)?;
            json!({"taskId": id.to_string(), "completion": completion.map(|value| json!({
                "status": "completed", "value": value.value, "completedAtMs": value.completed_at_ms,
            }))})
        }
        DurableCommand::Prune { older_than_secs, limit } => {
            let now_ms =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
            let retention_ms = u128::from(older_than_secs) * 1_000;
            let before_ms = u64::try_from(now_ms.saturating_sub(retention_ms))?;
            let deleted = store.prune_completed(before_ms, limit as usize)?;
            json!({"deleted": deleted, "beforeMs": before_ms, "limit": limit})
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
