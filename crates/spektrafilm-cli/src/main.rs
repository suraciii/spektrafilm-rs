//! The f32 production binary: one thin entry point over the shared CLI.

fn main() -> anyhow::Result<()> {
    spektrafilm_cli::run()
}
