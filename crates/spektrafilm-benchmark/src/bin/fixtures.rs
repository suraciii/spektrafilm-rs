use clap::Parser;
use spektrafilm_benchmark::{Result, fixtures};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "spektrafilm-bench-fixtures",
    about = "Generate packaged benchmark suites and targets.",
    after_help = "The generated bundle is consumed by `spektrafilm-bench run --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`."
)]
struct Args {
    #[arg(
        long,
        help = "Renderer executable to use for preset export and configuration validation"
    )]
    renderer: PathBuf,
    #[arg(long, help = "Explicit renderer data/profile directory")]
    data_dir: PathBuf,
    #[arg(long, help = "New fixture bundle directory; must not exist")]
    out: PathBuf,
    #[arg(
        long,
        conflicts_with = "download_natural",
        help = "Optional pinned natural RAW input to include"
    )]
    natural_input: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with = "natural_input",
        help = "Download the pinned natural RAW input"
    )]
    download_natural: bool,
}
fn main() {
    let args = Args::parse();
    let result: Result<_> = fixtures::create_fixtures(
        &args.renderer,
        &args.data_dir,
        &args.out,
        args.natural_input.as_deref(),
        args.download_natural,
    );
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => {
            eprintln!("{error}");
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"command":"fixtures","execution_status":"failed","errors":[error.to_value()]})
            );
            std::process::exit(1)
        }
    }
}
