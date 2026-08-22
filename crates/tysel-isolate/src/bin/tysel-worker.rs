fn main() {
    if let Some(output) =
        tysel_distribution::metadata_output("tysel-worker", env!("CARGO_PKG_VERSION"))
    {
        println!("{output}");
        return;
    }
    if let Err(err) = tysel_isolate::worker_main() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
