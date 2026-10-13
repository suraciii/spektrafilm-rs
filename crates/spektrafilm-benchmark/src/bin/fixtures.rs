use clap::Parser;
use spektrafilm_benchmark::{Result, fixtures};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "spektrafilm-bench-fixtures",
    about = "Generate packaged benchmark suites and targets."
)]
struct Args {
    #[arg(long)]
    renderer: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    natural_input: Option<PathBuf>,
    #[arg(long)]
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
