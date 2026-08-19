fn main() {
    if let Err(err) = tysel_isolate::worker_main() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
