use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::{auth, github, repo, server, session, state, worktree};

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
    Doctor {
        #[arg(long)]
        bundle: bool,
    },
    Open { pr: u64 },
    /// Configure the default editor in ~/.config/tldr/config.toml.
    /// `<name>` is one of vscode|cursor|zed|jetbrains|neovim, or a custom command;
    /// if custom, `--args` must contain `{path}`.
    Editor {
        name: String,
        #[arg(long)]
        args: Option<String>,
    },
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
        Some(Command::Doctor { bundle }) => crate::doctor::run(bundle).await,
        Some(Command::Open { pr }) => open_cmd(pr).await,
        Some(Command::Editor { name, args }) => editor_cmd(name, args).await,
        None => {
            println!("usage: tldr <pr-number>   (see --help)");
            Ok(())
        }
    }
}

async fn run(pr: u64, foreground: bool) -> Result<()> {
    let repo = repo::find_from_cwd().context("not in a git repo")?;
    let slug = repo.slug().context("could not determine GitHub slug")?;

    // If an existing live server is registered, route to it.
    let sess_path = session::session_path(&slug)?;
    if sess_path.exists() {
        match session::load(&sess_path) {
            Ok(s) if session::is_alive(&s) => {
                let http = reqwest::Client::new();
                let url = format!("http://127.0.0.1:{}/api/session/open-pr", s.port);
                let _ = http.post(&url)
                    .header("x-tldr-csrf", &s.csrf_token)
                    .json(&serde_json::json!({"pr_number": pr}))
                    .send().await;
                let browse = format!("http://127.0.0.1:{}/pr/{}/files", s.port, pr);
                open::that(&browse).ok();
                println!("routed to running tldr: {browse}");
                return Ok(());
            }
            _ => { session::remove(&sess_path); }
        }
    }

    let token = auth::token().await.context("no GitHub token (run `gh auth login` or set GITHUB_TOKEN)")?;

    tracing::info!(slug = %slug, pr_number = pr, "pr.open");
    let client = github::Client::new(token.clone());
    let meta = client.fetch_pr(&slug, pr).await?;

    tracing::info!("fetching ref");
    worktree::fetch_pr_ref(&repo.root, pr)?;
    let wt_path = state::worktree_path(&slug, pr)?;
    worktree::ensure_worktree(&repo.root, &wt_path, pr)?;

    let port = server::pick_port();
    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}/pr/{pr}/files");

    let ctx = server::ServerCtx::new(repo.clone(), slug.clone(), wt_path, meta, token, port);

    if !foreground {
        open::that(&url).ok();
    }

    tracing::info!(%url, "starting server");
    if !foreground {
        println!("tldr running: {url}");
    }
    server::serve(addr, ctx).await
}

async fn list() -> Result<()> {
    let paths = session::all_session_paths()?;
    let mut any_alive = false;
    for p in &paths {
        if let Ok(s) = session::load(p) {
            if session::is_alive(&s) {
                any_alive = true;
                println!("{}  pid={}  port={}  active_prs={:?}", s.slug, s.pid, s.port, s.active_prs);
            } else {
                session::remove(p);
            }
        }
    }
    if !any_alive {
        let token = match auth::token().await {
            Ok(t) => t,
            Err(_) => { println!("no active sessions; auth required to list open PRs"); return Ok(()); }
        };
        let http = reqwest::Client::new();
        let url = "https://api.github.com/search/issues?q=is:open+is:pr+review-requested:@me";
        let resp = http.get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "tldr-cli")
            .header("Accept", "application/vnd.github+json")
            .send().await?;
        let v: serde_json::Value = resp.json().await?;
        if let Some(items) = v["items"].as_array() {
            println!("open PRs awaiting your review:");
            for it in items {
                println!("  {} - {}", it["html_url"].as_str().unwrap_or(""), it["title"].as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

async fn status(_pr: Option<u64>) -> Result<()> {
    let repo = repo::find_from_cwd()?;
    let slug = repo.slug()?;
    let p = session::session_path(&slug)?;
    if !p.exists() { println!("no session for {slug}"); return Ok(()); }
    let s = session::load(&p)?;
    let alive = session::is_alive(&s);
    println!("slug: {}", s.slug);
    println!("pid: {} ({})", s.pid, if alive { "alive" } else { "dead" });
    println!("port: {}", s.port);
    println!("started_at: {}", s.started_at);
    println!("active_prs: {:?}", s.active_prs);
    Ok(())
}

async fn stop(_pr: Option<u64>, all: bool) -> Result<()> {
    let paths = if all {
        session::all_session_paths()?
    } else {
        let repo = repo::find_from_cwd()?;
        let slug = repo.slug()?;
        vec![session::session_path(&slug)?]
    };
    let http = reqwest::Client::new();
    for p in &paths {
        if !p.exists() { continue; }
        let s = match session::load(p) { Ok(s) => s, Err(_) => { session::remove(p); continue; } };
        if !session::is_alive(&s) { session::remove(p); continue; }
        let url = format!("http://127.0.0.1:{}/api/session/shutdown", s.port);
        match http.post(&url).header("x-tldr-csrf", &s.csrf_token).send().await {
            Ok(_) => println!("stopped {} (pid {})", s.slug, s.pid),
            Err(e) => println!("failed to stop {}: {}", s.slug, e),
        }
    }
    Ok(())
}

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

async fn open_cmd(pr: u64) -> Result<()> {
    let repo = repo::find_from_cwd()?;
    let slug = repo.slug()?;
    let p = session::session_path(&slug)?;
    if !p.exists() { return Err(anyhow::anyhow!("no active session")); }
    let s = session::load(&p)?;
    if !session::is_alive(&s) { session::remove(&p); return Err(anyhow::anyhow!("no active session")); }
    let target = if s.active_prs.contains(&pr) || pr != 0 {
        format!("http://127.0.0.1:{}/pr/{}/files", s.port, pr)
    } else if s.active_prs.len() == 1 {
        format!("http://127.0.0.1:{}/pr/{}/files", s.port, s.active_prs[0])
    } else {
        format!("http://127.0.0.1:{}/", s.port)
    };
    open::that(&target).ok();
    println!("{target}");
    Ok(())
}

async fn editor_cmd(name: String, args: Option<String>) -> Result<()> {
    use crate::editor::{parse_name, Editor};
    let ed = match (parse_name(&name), args) {
        (Ok(ed), None) => ed,
        (_, Some(tpl)) => {
            if !tpl.contains("{path}") {
                anyhow::bail!("--args template must contain `{{path}}`");
            }
            Editor::Custom { command: name, args_template: tpl }
        }
        (Err(e), None) => return Err(e),
    };
    let mut cfg = crate::config::Config::load().unwrap_or_default();
    cfg.editor = Some(ed);
    let p = cfg.save()?;
    println!("editor configured; wrote {}", p.display());
    Ok(())
}
