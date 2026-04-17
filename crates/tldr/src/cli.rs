use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::{auth, github, repo, server, state, worktree};

#[derive(Parser, Debug)]
#[command(name = "tldr", version, about = "Local-first PR review")]
pub struct Cli {
    /// PR number shortcut: `tldr 123`
    pub pr: Option<u64>,
    #[arg(long)]
    pub foreground: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Run { pr: u64, #[arg(long)] foreground: bool },
    List,
    Status { pr: Option<u64> },
    Stop { pr: Option<u64>, #[arg(long)] all: bool },
    Auth { #[command(subcommand)] cmd: AuthCmd },
    Config { #[command(subcommand)] cmd: ConfigCmd },
    Doctor,
    Open { pr: u64 },
    Editor { pr: u64 },
}

#[derive(Subcommand, Debug)]
pub enum AuthCmd { Status, Login, Logout }

#[derive(Subcommand, Debug)]
pub enum ConfigCmd { Show, Set { key: String, value: String } }

pub async fn dispatch(cli: Cli) -> Result<()> {
    if let Some(pr) = cli.pr {
        return run(pr, cli.foreground).await;
    }
    match cli.command {
        Some(Command::Run { pr, foreground }) => run(pr, foreground).await,
        Some(Command::List) => list().await,
        Some(Command::Status { pr }) => status(pr).await,
        Some(Command::Stop { pr, all }) => stop(pr, all).await,
        Some(Command::Auth { cmd }) => auth_cmd(cmd).await,
        Some(Command::Config { cmd }) => config_cmd(cmd).await,
        Some(Command::Doctor) => doctor().await,
        Some(Command::Open { pr }) => open_cmd(pr).await,
        Some(Command::Editor { pr }) => editor_cmd(pr).await,
        None => {
            println!("usage: tldr <pr-number>   (see --help)");
            Ok(())
        }
    }
}

async fn run(pr: u64, foreground: bool) -> Result<()> {
    let repo = repo::find_from_cwd().context("not in a git repo")?;
    let slug = repo.slug().context("could not determine GitHub slug")?;
    let token = auth::token().await.context("no GitHub token (run `gh auth login` or set GITHUB_TOKEN)")?;

    tracing::info!(?slug, pr, "fetching PR metadata");
    let client = github::Client::new(token.clone());
    let meta = client.fetch_pr(&slug, pr).await?;

    tracing::info!("fetching ref");
    worktree::fetch_pr_ref(&repo.root, pr)?;
    let wt_path = state::worktree_path(&slug, pr)?;
    worktree::ensure_worktree(&repo.root, &wt_path, pr)?;

    let port = server::pick_port();
    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}/pr/{pr}/files");

    let ctx = server::ServerCtx::new(repo.clone(), slug.clone(), wt_path, meta, token);

    if !foreground {
        // Detach: spawn in background and open browser.
        open::that(&url).ok();
    }

    tracing::info!(%url, "starting server");
    if !foreground {
        println!("tldr running: {url}");
    }
    server::serve(addr, ctx).await
}

async fn list() -> Result<()> { println!("(stub) active sessions: read from state dir"); Ok(()) }
async fn status(_pr: Option<u64>) -> Result<()> { println!("(stub) status"); Ok(()) }
async fn stop(_pr: Option<u64>, _all: bool) -> Result<()> { println!("(stub) stop"); Ok(()) }

async fn auth_cmd(cmd: AuthCmd) -> Result<()> {
    match cmd {
        AuthCmd::Status => {
            match auth::token().await {
                Ok(_) => println!("authenticated"),
                Err(e) => println!("not authenticated: {e}"),
            }
        }
        AuthCmd::Login => println!("run `gh auth login` or set GITHUB_TOKEN"),
        AuthCmd::Logout => println!("(stub) logout"),
    }
    Ok(())
}

async fn config_cmd(cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Show => {
            let c = crate::config::Config::load()?;
            println!("{}", toml::to_string_pretty(&c)?);
        }
        ConfigCmd::Set { key, value } => println!("(stub) set {key}={value}"),
    }
    Ok(())
}

async fn doctor() -> Result<()> {
    println!("git: {}", which::which("git").map(|p| p.display().to_string()).unwrap_or_else(|_| "MISSING".into()));
    println!("gh: {}", which::which("gh").map(|p| p.display().to_string()).unwrap_or_else(|_| "MISSING".into()));
    println!("token: {}", if auth::token().await.is_ok() { "ok" } else { "missing" });
    Ok(())
}

async fn open_cmd(pr: u64) -> Result<()> {
    // TODO: look up running server port from state dir. For now hint.
    println!("(stub) look up server for PR {pr} in state dir and open browser");
    Ok(())
}

async fn editor_cmd(pr: u64) -> Result<()> {
    let repo = repo::find_from_cwd()?;
    let slug = repo.slug().context("slug")?;
    let wt = state::worktree_path(&slug, pr)?;
    println!("{}", wt.display());
    Ok(())
}
