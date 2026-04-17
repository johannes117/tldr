use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = tldr::cli::Cli::parse();
    let foreground = cli.foreground
        || matches!(&cli.command, Some(tldr::cli::Command::Run { foreground: true, .. }));
    let _guard = tldr::logging::init(foreground).ok();
    let result = tldr::cli::dispatch(cli).await;
    if let Err(e) = &result {
        tracing::error!(error = %e, backtrace = ?e.backtrace(), "command failed");
    }
    result
}
