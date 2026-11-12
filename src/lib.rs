mod adapters;
mod agent;
mod core;
mod join;
mod protocol;
mod share;
#[cfg(test)]
mod tests;

use std::{
    error::Error,
    ffi::OsString,
    io::{self, BufRead, Read},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use hyper::Uri;
use iroh::RelayUrl;
use tokio::signal::unix::{SignalKind, signal};

use crate::{
    adapters::{
        endpoint::{Discovery, NetConfig},
        keyfile, runtime,
        t3::{self, T3Target},
    },
    agent::guides::{self, Kind},
    core::{
        ids::{CommentId, Sid},
        name::JoinerName,
        token::{PartyToken, TokenError},
    },
    join::{JoinConfig, Joiner},
    protocol::control::AnswerText,
    share::{
        Share, ShareConfig, Source,
        delivery::{CommandTarget, Target},
        proxy::UpstreamUrl,
    },
};

pub(crate) type BoxError = Box<dyn Error + Send + Sync>;

/// How long a public share waits to reach a relay before printing its token.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
#[command(
    name = "jamjam",
    version,
    about = "Share a site under development and collect comments on it"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Share a static directory (live reload); print the party token.
    Serve {
        /// Directory to share.
        #[arg(default_value = ".")]
        directory: PathBuf,
        #[command(flatten)]
        net: ShareNet,
        #[command(flatten)]
        delivery: Delivery,
    },

    /// Share a running development server; print the party token.
    Proxy {
        /// Base URL of the development server.
        upstream: UpstreamUrl,
        #[command(flatten)]
        net: ShareNet,
        #[command(flatten)]
        delivery: Delivery,
    },

    /// Join a share and open it in the browser.
    Join {
        /// The party token printed by `jamjam serve` or `jamjam proxy`, or
        /// `-` to read it from stdin (out of `ps` and shell history).
        #[arg(value_parser = parse_token)]
        token: TokenArg,
        /// Display name shown next to your comments (default: user@host).
        #[arg(long)]
        name: Option<JoinerName>,
        /// Do not open the browser.
        #[arg(long)]
        no_open: bool,
        /// Dial the share at this address, without relays or address lookup.
        #[arg(long, hide = true)]
        share_addr: Vec<SocketAddr>,
    },

    /// List the guides for agents, or print one.
    #[command(hide = true)]
    Agent { kind: Option<Kind> },

    /// Wait until comments are pending, print them as NDJSON, exit.
    #[command(hide = true)]
    Recv {
        #[command(flatten)]
        which: Which,
    },

    /// Answer on the page (Markdown; read from stdin when not given).
    #[command(hide = true)]
    Send {
        message: Option<String>,
        /// Show the answer next to this comment.
        #[arg(long, value_name = "COMMENT ID")]
        anchor: Option<CommentId>,
        #[command(flatten)]
        which: Which,
    },
}

#[derive(Debug, Args)]
struct ShareNet {
    /// No relays or address lookup: bind 127.0.0.1 and print the address
    /// for `jamjam join --share-addr`.
    #[arg(long, hide = true)]
    local: bool,
}

/// Where comments go: the t3 target, a command, or (neither) `jamjam recv`.
#[derive(Debug, Args)]
struct Delivery {
    /// T3 thread to deliver comments to (default: $T3CODE_THREAD_ID with --t3-url).
    #[arg(long)]
    thread: Option<String>,
    /// T3's MCP URL (t3 target).
    #[arg(long, value_name = "URL")]
    t3_url: Option<String>,
    /// File holding the T3 token (t3 target).
    #[arg(long, value_name = "FILE")]
    t3_token_file: Option<PathBuf>,
    /// Seconds a command may take per batch before it is killed.
    #[arg(long, value_name = "SECS", default_value_t = 60)]
    command_timeout: u64,
    /// Hand each batch of comments to this command, as NDJSON on its stdin.
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<OsString>,
}

/// Where `jamjam join` gets its token.
#[derive(Clone, Debug)]
enum TokenArg {
    Given(PartyToken),
    /// `-`: the first line of stdin.
    Stdin,
}

fn parse_token(raw: &str) -> Result<TokenArg, String> {
    if raw == "-" {
        Ok(TokenArg::Stdin)
    } else {
        raw.parse()
            .map(TokenArg::Given)
            .map_err(|error: TokenError| error.to_string())
    }
}

impl TokenArg {
    fn resolve(self, stdin: impl BufRead) -> Result<PartyToken, BoxError> {
        match self {
            Self::Given(token) => Ok(token),
            Self::Stdin => {
                let mut line = String::new();
                stdin.take(1024).read_line(&mut line)?;
                Ok(line
                    .parse()
                    .map_err(|error| format!("the token on stdin: {error}"))?)
            }
        }
    }
}

/// Which running share `recv` and `send` talk to.
#[derive(Debug, Args)]
struct Which {
    /// The share's id (printed at startup); needed only when several
    /// shares run.
    #[arg(long, value_name = "ID")]
    share: Option<Sid>,
}

pub async fn run(cli: Cli) -> Result<(), BoxError> {
    let Some(command) = cli.command else {
        print!("{}", guides::OVERVIEW);
        return Ok(());
    };
    match command {
        Command::Serve {
            directory,
            net,
            delivery,
        } => {
            let root = std::fs::canonicalize(&directory).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not share {}: {error}", directory.display()),
                )
            })?;
            if !root.is_dir() {
                return Err(format!("{} is not a directory", root.display()).into());
            }
            let site = root.display().to_string();
            share(Source::Files(root), &net, delivery, &site, " (live reload)").await
        }
        Command::Proxy {
            upstream,
            net,
            delivery,
        } => {
            let site = upstream.to_string();
            share(Source::Proxy(upstream), &net, delivery, &site, "").await
        }
        Command::Join {
            token,
            name,
            no_open,
            share_addr,
        } => {
            let token = token.resolve(io::stdin().lock())?;
            join(token, name, no_open, share_addr).await
        }
        Command::Agent { kind } => {
            match kind {
                None => print!("{}", guides::LIST),
                Some(kind) => print!("{}", guides::guide(kind)),
            }
            Ok(())
        }
        Command::Recv { which } => {
            let stream = runtime::connect(&runtime::default_dir()?, which.share.as_ref()).await?;
            agent::recv(stream, &mut io::stdout().lock()).await?;
            Ok(())
        }
        Command::Send {
            message,
            anchor,
            which,
        } => {
            let message = match message {
                Some(message) => message,
                None => {
                    let mut message = String::new();
                    io::stdin().read_to_string(&mut message)?;
                    message
                }
            };
            let message = AnswerText::try_from(message)?;
            let stream = runtime::connect(&runtime::default_dir()?, which.share.as_ref()).await?;
            agent::send(stream, message, anchor).await
        }
    }
}

/// The target the delivery options select, and a note for the startup
/// output; `site` is what is shared, `env_thread` is `T3CODE_THREAD_ID`.
/// jamjam probes nothing: missing pieces are errors naming the option.
fn target(
    delivery: Delivery,
    site: &str,
    env_thread: Option<OsString>,
) -> Result<(Target, Option<String>), BoxError> {
    let Delivery {
        thread,
        t3_url,
        t3_token_file,
        command_timeout,
        command,
    } = delivery;
    let t3 = thread.is_some() || t3_url.is_some() || t3_token_file.is_some();
    if t3 && !command.is_empty() {
        return Err(
            "choose the t3 target (--thread, --t3-url) or a command after --, not both".into(),
        );
    }
    if t3 {
        let url = t3_url.ok_or("the t3 target needs T3's MCP URL: pass --t3-url")?;
        let url = url
            .parse::<Uri>()
            .ok()
            .filter(|url| matches!(url.scheme_str(), Some("http" | "https")))
            .ok_or_else(|| format!("--t3-url {url} is not an http:// or https:// URL"))?;
        if url.scheme_str() == Some("http") && !url.host().is_some_and(is_loopback) {
            return Err(format!(
                "--t3-url {url}: plain http:// only to this machine (localhost, 127.0.0.1, [::1]); use https://"
            )
            .into());
        }
        let token_file = t3_token_file
            .ok_or("the t3 target needs a file holding the T3 token: pass --t3-token-file")?;
        let thread = match thread {
            Some(thread) => thread,
            None => env_thread
                .and_then(|thread| thread.into_string().ok())
                .filter(|thread| !thread.is_empty())
                .ok_or("the t3 target needs a thread: pass --thread (or set T3CODE_THREAD_ID)")?,
        };
        let target = T3Target {
            url,
            token_file,
            thread,
            site: site.to_owned(),
        };
        return Ok((Target::T3(target), None));
    }
    if !command.is_empty() {
        let command = CommandTarget {
            argv: command,
            timeout: Duration::from_secs(command_timeout),
        };
        return Ok((Target::Command(command), None));
    }
    let note = env_thread.map(|_| {
        "T3CODE_THREAD_ID is set, but without --t3-url comments wait for jamjam recv".to_owned()
    });
    Ok((Target::Recv, note))
}

/// Whether a URL's host is this machine.
fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn net_config(discovery: Discovery) -> Result<NetConfig, BoxError> {
    let mut net = NetConfig::new(discovery);
    if let Some(relay) = std::env::var_os("JAMJAM_RELAY").filter(|relay| !relay.is_empty()) {
        let relay = relay.to_str().ok_or("JAMJAM_RELAY is not valid UTF-8")?;
        net.relay = Some(
            relay
                .parse::<RelayUrl>()
                .map_err(|error| format!("JAMJAM_RELAY: {error}"))?,
        );
    }
    Ok(net)
}

async fn share(
    source: Source,
    net: &ShareNet,
    delivery: Delivery,
    site: &str,
    detail: &str,
) -> Result<(), BoxError> {
    let discovery = if net.local {
        Discovery::Direct
    } else {
        Discovery::Public
    };
    let (mut target, note) = target(delivery, site, std::env::var_os("T3CODE_THREAD_ID"))?;
    let goes_to = match &mut target {
        Target::Recv => "jamjam recv".to_owned(),
        Target::Command(command) => format!(
            "the command {}",
            command
                .argv
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Target::T3(t3) => {
            let thread = t3::open(t3).await?;
            let resolved = thread
                .resolved_from
                .map(|child| format!(", the top-level thread of {child}"))
                .unwrap_or_default();
            let goes_to = format!("the T3 thread {:?} ({}{resolved})", thread.title, thread.id);
            t3.thread = thread.id;
            goes_to
        }
    };
    let config = ShareConfig::new(
        source,
        net_config(discovery)?,
        target,
        runtime::default_dir()?,
    );
    let share = Share::start(config).await?;
    if discovery == Discovery::Public
        && tokio::time::timeout(ONLINE_TIMEOUT, share.endpoint().online())
            .await
            .is_err()
    {
        tracing::warn!("no relay reachable yet; joiners may not find this share");
    }
    println!("jamjam: sharing {site}{detail}");
    println!("jamjam: token: {}", share.token());
    println!(
        "jamjam: warning: anyone with the token can see the site and send comments to the agent until this share stops"
    );
    println!("jamjam: share id: {}", share.id());
    println!("jamjam: comments go to {goes_to}");
    if let Some(note) = note {
        println!("jamjam: note: {note}");
    }
    println!("jamjam: agents: run jamjam agent for the guides");
    if discovery == Discovery::Direct {
        for addr in share.direct_addrs() {
            println!("jamjam: direct address: {addr}");
        }
    }
    shutdown_signal().await?;
    share.stop().await;
    Ok(())
}

async fn join(
    token: PartyToken,
    name: Option<JoinerName>,
    no_open: bool,
    share_addrs: Vec<SocketAddr>,
) -> Result<(), BoxError> {
    let discovery = if share_addrs.is_empty() {
        Discovery::Public
    } else {
        Discovery::Direct
    };
    let name = name.unwrap_or_else(default_name);
    let key = keyfile::load_or_create(&keyfile::default_path()?)?;
    let joiner = Joiner::start(JoinConfig {
        token,
        name,
        key,
        net: net_config(discovery)?,
        share_addrs,
    })
    .await?;
    let url = joiner.url();
    println!("jamjam: open {url}");
    if !no_open {
        open_browser(&url);
    }
    let mut view = joiner.watch();
    let refused = tokio::select! {
        refused = view.wait_for(|view| view.refused.is_some()) => {
            refused.ok().and_then(|view| view.refused)
        }
        signal = shutdown_signal() => {
            signal?;
            None
        }
    };
    joiner.stop().await;
    match refused {
        Some(reason) => Err(format!(
            "the share refused this joiner ({reason:?}): the token is wrong or from an earlier run"
        )
        .into()),
        None => Ok(()),
    }
}

fn default_name() -> JoinerName {
    let user = std::env::var("USER").unwrap_or_default();
    let host = nix::unistd::gethostname()
        .map(|host| host.to_string_lossy().into_owned())
        .unwrap_or_default();
    JoinerName::default_for(&user, &host)
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let spawned = std::process::Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not open the browser; open the address yourself");
    }
}

async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
        _ = hangup.recv() => Ok(()),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
