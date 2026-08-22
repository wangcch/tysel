use std::collections::BTreeSet;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};

use crate::project::ProjectContext;

pub fn run(project: &ProjectContext, name: Option<&str>, list: bool) -> Result<()> {
    if list || name.is_none() {
        return print_tasks(project);
    }
    let name = name.expect("checked above");
    if !project.manifest.tasks.contains_key(name) {
        let available = project.manifest.tasks.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(anyhow!(
            "unknown task {name:?}; available tasks: {}",
            if available.is_empty() { "none" } else { &available }
        ));
    }

    let mut completed = BTreeSet::new();
    execute_task(project, name, &mut completed)?;
    println!("task {name} completed");
    Ok(())
}

fn print_tasks(project: &ProjectContext) -> Result<()> {
    println!("Tasks in {}", project.manifest_path.display());
    if project.manifest.tasks.is_empty() {
        println!("  (none)");
        return Ok(());
    }
    for (name, task) in &project.manifest.tasks {
        let description = task.description.as_deref().unwrap_or("");
        if description.is_empty() {
            println!("  {name}");
        } else {
            println!("  {name:<16} {description}");
        }
        if !task.depends.is_empty() {
            println!("    depends: {}", task.depends.join(", "));
        }
    }
    Ok(())
}

fn execute_task(
    project: &ProjectContext,
    name: &str,
    completed: &mut BTreeSet<String>,
) -> Result<()> {
    if completed.contains(name) {
        return Ok(());
    }
    let task = &project.manifest.tasks[name];
    for dependency in &task.depends {
        execute_task(project, dependency, completed)?;
    }

    for (index, step) in task.steps.iter().enumerate() {
        println!("task {name} [{}/{}] tysel {}", index + 1, task.steps.len(), step.join(" "));
        let executable = std::env::current_exe().context("resolve current Tysel executable")?;
        let mut command = Command::new(executable);
        command
            .arg(&step[0])
            .args(&step[1..])
            .arg("--manifest")
            .arg(&project.manifest_path)
            .current_dir(&project.root)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status =
            command.status().with_context(|| format!("run task {name:?} step {}", index + 1))?;
        if !status.success() {
            return Err(anyhow!("task {name:?} failed at step {} with status {status}", index + 1));
        }
    }
    completed.insert(name.to_owned());
    Ok(())
}
