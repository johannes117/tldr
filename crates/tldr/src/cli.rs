use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::{auth, github, repo, secrets, server, session, state, worktree};
use std::io::{self, BufRead, Write};

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
    Auth { #[command(subcommand)] cmd: Option<AuthCmd> },
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
    /// Interactive first-time setup: GitHub auth + Anthropic API key.
    Init {
        /// Skip interactive prompts; use existing env/config.
        #[arg(long)]
        non_interactive: bool,
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
        Some(Command::Init { non_interactive }) => init_cmd(non_interactive).await,
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

async fn auth_cmd(cmd: Option<AuthCmd>) -> Result<()> {
    let cmd = match cmd {
        Some(c) => c,
        None => {
            println!("usage: tldr auth <login|logout|status>");
            println!("  login   Sign in via GitHub device flow");
            println!("  logout  Remove stored credentials");
            println!("  status  Show current authentication status");
            return Ok(());
        }
    };
    match cmd {
        AuthCmd::Status => {
            match auth::get_token_with_source().await {
                Ok((tok, src)) => match auth::fetch_login(&tok).await {
                    Ok(login) => println!("Signed in as @{login} (source: {})", src.as_str()),
                    Err(e) => println!("token present (source: {}) but /user failed: {e}", src.as_str()),
                },
                Err(_) => println!("Not signed in"),
            }
        }
        AuthCmd::Login => {
            let tok = auth::device_login().await?;
            auth::store_token(&tok)?;
            let login = auth::fetch_login(&tok).await.unwrap_or_else(|_| "unknown".into());
            println!("Signed in as @{login}");
        }
        AuthCmd::Logout => {
            auth::clear_token()?;
            println!("Signed out");
        }
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

async fn init_cmd(non_interactive: bool) -> Result<()> {
    println!("tldr init — first-time setup");
    println!();

    // 1. GitHub auth
    match auth::get_token_with_source().await {
        Ok((tok, src)) => {
            let login = auth::fetch_login(&tok).await.unwrap_or_else(|_| "unknown".into());
            println!("  [ok] GitHub: signed in as @{login} (source: {})", src.as_str());
        }
        Err(_) => {
            if non_interactive {
                println!("  [!] GitHub: not signed in. Run `tldr auth login`.");
            } else {
                print!("  [!] GitHub: not signed in. Sign in now via device flow? [Y/n] ");
                io::stdout().flush().ok();
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                let ans = line.trim().to_lowercase();
                if ans.is_empty() || ans == "y" || ans == "yes" {
                    let tok = auth::device_login().await?;
                    auth::store_token(&tok)?;
                    let login = auth::fetch_login(&tok).await.unwrap_or_else(|_| "unknown".into());
                    println!("  [ok] GitHub: signed in as @{login}");
                } else {
                    println!("  skipping GitHub sign-in; run `tldr auth login` later.");
                }
            }
        }
    }

    // 2. Anthropic API key
    let existing_env = std::env::var("ANTHROPIC_API_KEY").ok().filter(|s| !s.is_empty());
    let existing_stored = secrets::read("anthropic_api_key").ok().flatten();
    if existing_env.is_some() {
        println!("  [ok] Anthropic: using ANTHROPIC_API_KEY from environment");
    } else if existing_stored.is_some() {
        println!("  [ok] Anthropic: API key already stored in keychain");
    } else if non_interactive {
        println!("  [!] Anthropic: no API key configured. Set ANTHROPIC_API_KEY or re-run `tldr init`.");
    } else {
        println!();
        println!("  Enter your Anthropic API key (get one at https://console.anthropic.com/).");
        print!("  Key (leave blank to skip): ");
        io::stdout().flush().ok();
        let key = read_secret_line()?;
        let key = key.trim();
        if key.is_empty() {
            println!("  skipped. AI features will be disabled until a key is configured.");
        } else {
            secrets::store("anthropic_api_key", key)?;
            println!("  [ok] Anthropic: API key stored in keychain");
        }
    }

    // 3. Config: enable AI, save defaults
    let mut cfg = crate::config::Config::load().unwrap_or_default();
    let has_key = existing_env.is_some()
        || existing_stored.is_some()
        || secrets::read("anthropic_api_key").ok().flatten().is_some();
    if has_key {
        cfg.ai.enabled = true;
    }
    let p = cfg.save()?;
    println!();
    println!("  wrote config: {}", p.display());
    println!();
    println!("Setup complete. Try: `tldr <pr-number>` from inside a git repo.");
    Ok(())
}

fn read_secret_line() -> Result<String> {
    // Best-effort no-echo on unix; on failure, fall back to echoed stdin.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let stdin = io::stdin();
        let fd = stdin.as_raw_fd();
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut term) == 0 {
                let orig = term;
                term.c_lflag &= !libc::ECHO;
                libc::tcsetattr(fd, libc::TCSANOW, &term);
                let mut line = String::new();
                let res = stdin.lock().read_line(&mut line);
                libc::tcsetattr(fd, libc::TCSANOW, &orig);
                println!();
                return res.map(|_| line).map_err(Into::into);
            }
        }
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line)
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
