fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let options = options()?;
    let stub = tysel_testkit::find_stub()?;
    let report = tysel_testkit::measure(&stub)?;
    print!("{}", tysel_testkit::format_report(&report));
    if !tysel_testkit::gates_passed(&report) {
        anyhow::bail!("one or more §30 gates failed");
    }
    if let Some(path) = options.evidence {
        let source_commit = options
            .source_commit
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--source-commit is required with --evidence"))?;
        let command = options
            .command
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--command is required with --evidence"))?;
        let evidence = tysel_testkit::benchmark_evidence(&report, source_commit, command)?;
        tysel_testkit::write_benchmark_evidence(&path, &evidence)?;
        println!("Evidence             {}", path.display());
    } else if options.source_commit.is_some() || options.command.is_some() {
        anyhow::bail!("--source-commit and --command require --evidence");
    }
    Ok(())
}

#[derive(Default)]
struct Options {
    evidence: Option<std::path::PathBuf>,
    source_commit: Option<String>,
    command: Option<String>,
}

fn options() -> anyhow::Result<Options> {
    let mut options = Options::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing value for {}", argument.to_string_lossy()))?;
        match argument.to_str() {
            Some("--evidence") if options.evidence.is_none() => {
                options.evidence = Some(value.into());
            }
            Some("--source-commit") if options.source_commit.is_none() => {
                options.source_commit = Some(
                    value
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("--source-commit is not valid UTF-8"))?,
                );
            }
            Some("--command") if options.command.is_none() => {
                options.command = Some(
                    value
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("--command is not valid UTF-8"))?,
                );
            }
            _ => anyhow::bail!("unknown or duplicate option {}", argument.to_string_lossy()),
        }
    }
    Ok(options)
}
