mod adapters;
mod core;
mod join;
mod protocol;
mod share;
#[cfg(test)]
mod tests;

use std::{error::Error, io, net::SocketAddr, path::PathBuf, process::Stdio, time::Duration};

use clap::{Args, Parser, Subcommand};
use tokio::signal::unix::{SignalKind, signal};

use crate::{
    adapters::{
        endpoint::{Discovery, NetConfig},
        keyfile,
    },
    core::{name::JoinerName, token::PartyToken},
    join::{JoinConfig, Joiner},
    share::{Share, ShareConfig, Source, proxy::UpstreamUrl},
};

pub(crate) type BoxError = Box<dyn Error + Send + Sync>;

/// How long a public share waits to reach a relay before printing its token.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
#[command(
    name = "jamjam",
    version,
    about = "Share a site under development and collect comments on it",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
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
    },

    /// Share a running development server; print the party token.
    Proxy {
        /// Base URL of the development server.
        upstream: UpstreamUrl,
        #[command(flatten)]
        net: ShareNet,
    },

    /// Join a share and open it in the browser.
    Join {
        /// The party token printed by `jamjam serve` or `jamjam proxy`.
        token: PartyToken,
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
}

#[derive(Debug, Args)]
struct ShareNet {
    /// No relays or address lookup: bind 127.0.0.1 and print the address
    /// for `jamjam join --share-addr`.
    #[arg(long, hide = true)]
    local: bool,
}

pub async fn run(cli: Cli) -> Result<(), BoxError> {
    match cli.command {
        Command::Serve { directory, net } => {
            let root = std::fs::canonicalize(&directory).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not share {}: {error}", directory.display()),
                )
            })?;
            if !root.is_dir() {
                return Err(format!("{} is not a directory", root.display()).into());
            }
            let what = format!("{} (live reload)", root.display());
            share(Source::Files(root), &net, &what).await
        }
        Command::Proxy { upstream, net } => {
            let what = upstream.to_string();
            share(Source::Proxy(upstream), &net, &what).await
        }
        Command::Join {
            token,
            name,
            no_open,
            share_addr,
        } => join(token, name, no_open, share_addr).await,
    }
}

async fn share(source: Source, net: &ShareNet, what: &str) -> Result<(), BoxError> {
    let discovery = if net.local {
        Discovery::Direct
    } else {
        Discovery::Public
    };
    let share = Share::start(ShareConfig::new(source, NetConfig::new(discovery))).await?;
    if discovery == Discovery::Public
        && tokio::time::timeout(ONLINE_TIMEOUT, share.endpoint().online())
            .await
            .is_err()
    {
        tracing::warn!("no relay reachable yet; joiners may not find this share");
    }
    println!("jamjam: sharing {what}");
    println!("jamjam: token: {}", share.token());
    println!("jamjam: anyone with the token can see the site and comment until this share stops");
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
        net: NetConfig::new(discovery),
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
mod cli_tests {
    use clap::Parser;

    use crate::{Cli, Command};

    #[test]
    fn parses_the_three_commands() {
        let serve = Cli::try_parse_from(["jamjam", "serve"]).unwrap();
        assert!(matches!(serve.command, Command::Serve { .. }));
        let proxy = Cli::try_parse_from(["jamjam", "proxy", "http://127.0.0.1:8000"]).unwrap();
        assert!(matches!(proxy.command, Command::Proxy { .. }));
        let token = format!("{}:{}", "ab".repeat(32), "a".repeat(26));
        let join = Cli::try_parse_from(["jamjam", "join", &token, "--name", "alice", "--no-open"])
            .unwrap();
        assert!(matches!(join.command, Command::Join { no_open: true, .. }));
        assert!(Cli::try_parse_from(["jamjam", "join", "nonsense"]).is_err());
        assert!(Cli::try_parse_from(["jamjam", "join", &token, "--name", "a b"]).is_err());
        assert!(Cli::try_parse_from(["jamjam", "http://127.0.0.1:8000"]).is_err());
        assert!(Cli::try_parse_from(["jamjam"]).is_err());
    }
}
