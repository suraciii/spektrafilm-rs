//! The f64 reference build of the shared CLI entry point.

#[path = "../main.rs"]
mod cli;

#[cfg(test)]
pub(crate) use cli::Cli;
pub(crate) use cli::Commands;

fn main() -> anyhow::Result<()> {
    cli::main()
}
