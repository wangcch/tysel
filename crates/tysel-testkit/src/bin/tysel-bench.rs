fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let stub = tysel_testkit::find_stub()?;
    let report = tysel_testkit::measure(&stub)?;
    print!("{}", tysel_testkit::format_report(&report));
    if !tysel_testkit::gates_passed(&report) {
        anyhow::bail!("one or more §30 gates failed");
    }
    Ok(())
}
