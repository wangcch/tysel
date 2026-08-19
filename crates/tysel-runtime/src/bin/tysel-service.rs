use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match tysel_runtime::run_stub().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}
