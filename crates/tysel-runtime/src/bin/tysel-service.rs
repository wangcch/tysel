use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some(output) =
        tysel_distribution::metadata_output("tysel-service", env!("CARGO_PKG_VERSION"))
    {
        println!("{output}");
        return ExitCode::SUCCESS;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: create Tokio runtime: {err}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(tysel_runtime::run_stub()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}
