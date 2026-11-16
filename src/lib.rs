mod adapters;
mod agent;
mod core;
mod join;
mod protocol;
mod share;
#[cfg(test)]
mod tests;

use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, BufRead, Read},
    net::SocketAddr,
    path::PathBuf,
    process::{Command as Process, Stdio},
    str::FromStr,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use iroh::RelayUrl;
use nix::unistd;
use rootcause::{
    option_ext::{NoneError, OptionExt},
    prelude::{ResultExt, bail},
};
use tokio::{
    signal::{
        ctrl_c,
        unix::{SignalKind, signal},
    },
    time,
};
use zeroize::Zeroizing;

use crate::{
    adapters::{
        endpoint::{Discovery, NetConfig, generate_key},
        runtime, settings,
    },
    agent::guides::{self, Kind},
    core::{
        ids::{CommentId, Sid},
        name::JoinerName,
        port::Port,
        rng::SystemRng,
        settings::{AnswerTimeout, KeyBinding, SettingsOverride},
        text::{self, Untrusted},
        token::{PartyToken, TokenError},
    },
    join::{JoinConfig, Joiner, ProxyLimits, Refused},
    protocol::{agent::Addressee, control::AnswerText},
    share::{
        Share, ShareConfig, Source,
        delivery::{CommandTarget, Target},
        proxy::UpstreamUrl,
    },
};

/// Errors are rootcause reports; `main` prints them with [`describe`].
pub use rootcause::Report;

/// Installs the log output: `jamparty: ...` lines on stderr, or fmt's
/// detailed format when RUST_LOG is set.
pub fn init_logging() {
    adapters::output::init();
}

/// An error for people: its chain of contexts on one line, outermost
/// first (`could not read <file>: permission denied`), control characters
/// escaped (an error may quote what a share or joiner sent). The marker an
/// `Option`'s `.context(…)` puts under its message ("Expected value of
/// type …, but found None") says nothing to people and is left out.
#[must_use]
pub fn describe(error: &Report) -> String {
    error
        .iter_reports()
        .filter(|report| report.downcast_current_context::<NoneError>().is_none())
        .map(|report| Untrusted(report.format_current_context()).to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// How long a page's request waits for the joiner to reach the share again
/// (after a drop) before it is answered "not reachable".
const CONNECT_WAIT: Duration = Duration::from_secs(15);
/// How long `jamparty join` waits for the share to answer its hello.
const ADMISSION_WAIT: Duration = Duration::from_secs(10);

/// How long a public share waits to reach a relay before printing its token.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
#[command(
    name = "jamparty",
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
        /// The party token printed by `jamparty serve` or `jamparty proxy`, or
        /// `-` to read it from stdin (out of `ps` and shell history).
        token: TokenArg,
        #[command(flatten)]
        settings: JoinSettings,
        /// Display name shown next to your comments (default: user@host).
        #[arg(long)]
        name: Option<JoinerName>,
        /// Do not open the browser.
        #[arg(long)]
        no_open: bool,
        /// Dial the share at this address, without relays or address lookup.
        #[arg(long, hide = true)]
        share_addr: Vec<SocketAddr>,
        /// Serve the site on this local port [default: the dev server's
        /// port when it is free here, else any free port]
        #[arg(long)]
        port: Option<Port>,
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
        /// Show the answer next to this comment (and, without --to, only
        /// to its author).
        #[arg(long, value_name = "COMMENT ID")]
        anchor: Option<CommentId>,
        /// Only to this person: a name, `key:<fingerprint>` (or the full
        /// key) when names collide, or `all` for everyone.
        #[arg(long, value_name = "NAME")]
        to: Option<Addressee>,
        #[command(flatten)]
        which: Which,
    },
}

#[derive(Debug, Args)]
struct ShareNet {
    /// No relays or address lookup: bind 127.0.0.1 and print the address
    /// for `jamparty join --share-addr`.
    #[arg(long, hide = true)]
    local: bool,
}

/// Deliver comments to a command, or hold them for `jamparty recv`.
#[derive(Debug, Args)]
struct Delivery {
    /// How long a command may take per batch before it is killed: seconds,
    /// or a number with s, m or h.
    #[arg(long, value_name = "SECS", default_value = "60", value_parser = timeout)]
    command_timeout: Duration,
    /// Hand each batch of comments to this command, as NDJSON on its stdin.
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<OsString>,
}

/// `--command-timeout`: whole seconds, or a number of seconds, minutes or
/// hours (`90`, `90s`, `2m`, `1h`); at least a second.
fn timeout(text: &str) -> Result<Duration, String> {
    let (number, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text, "s"),
    };
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => {
            return Err(format!(
                "{text:?}: give seconds, or a number with s, m or h"
            ));
        }
    };
    let secs = number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(scale))
        .filter(|secs| *secs > 0)
        .ok_or_else(|| format!("{text:?}: give a whole number of at least 1"))?;
    Ok(Duration::from_secs(secs))
}

/// Where `jamparty join` gets its token.
#[derive(Clone, Debug)]
enum TokenArg {
    Given(PartyToken),
    /// `-`: the first line of stdin.
    Stdin,
}

impl FromStr for TokenArg {
    type Err = TokenError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw == "-" {
            Ok(Self::Stdin)
        } else {
            raw.parse().map(Self::Given)
        }
    }
}

impl TokenArg {
    fn resolve(self, stdin: impl BufRead) -> Result<PartyToken, Report> {
        match self {
            Self::Given(token) => Ok(token),
            Self::Stdin => {
                // Wiped after parsing; sized for the whole read, so it never
                // grows and leaves a copy of the secret behind.
                let mut line = Zeroizing::new(String::with_capacity(TOKEN_LINE_LIMIT));
                stdin
                    .take(u64::try_from(TOKEN_LINE_LIMIT)?)
                    .read_line(&mut line)?;
                Ok(line.parse::<PartyToken>().context("the token on stdin")?)
            }
        }
    }
}

/// How much of stdin `jamparty join -` reads for the token.
const TOKEN_LINE_LIMIT: usize = 1024;

/// Which running share `recv` and `send` talk to.
#[derive(Debug, Args)]
struct Which {
    /// The share's id (printed at startup); needed only when several
    /// shares run.
    #[arg(long, value_name = "ID")]
    share: Option<Sid>,
}

pub async fn run(cli: Cli) -> Result<(), Report> {
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
            let root = fs::canonicalize(&directory)
                .context_with(|| format!("could not share {}", directory.display()))?;
            if !root.is_dir() {
                bail!("{} is not a directory", root.display());
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
            settings,
            name,
            no_open,
            share_addr,
            port,
        } => {
            let token = token.resolve(io::stdin().lock())?;
            let open = if no_open { Open::No } else { Open::Browser };
            join(token, name, open, share_addr, port, settings).await
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
            to,
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
            agent::send(stream, message, anchor, to).await
        }
    }
}

/// Select delivery; inside T3 (`in_t3`: T3CODE_THREAD_ID is set), suggest
/// the command target when none is given.
fn target(delivery: Delivery, source: &Source, in_t3: bool) -> (Target, Option<String>) {
    let Delivery {
        command_timeout,
        command,
    } = delivery;
    if !command.is_empty() {
        return (
            Target::Command(CommandTarget {
                argv: command,
                timeout: command_timeout,
            }),
            None,
        );
    }
    let note = in_t3.then(|| {
        format!(
            "inside T3, deliver to this thread with: jamparty {} -- t3 thread send \"$T3CODE_THREAD_ID\" -",
            source_args(source)
        )
    });
    (Target::Recv, note)
}

/// `source` as `jamparty` arguments to paste: the mode, then the directory
/// or URL quoted for the shell, or a placeholder when it cannot be shown
/// exactly (not UTF-8) or safely (control characters).
fn source_args(source: &Source) -> String {
    let (mode, placeholder, value) = match source {
        Source::Files(root) => ("serve", "<dir>", root.to_str().map(str::to_owned)),
        Source::Proxy(upstream) => ("proxy", "<url>", Some(upstream.to_string())),
    };
    let value = value
        .filter(|value| !value.chars().any(text::escaped))
        .map_or_else(|| placeholder.to_owned(), |value| shell_quote(&value));
    format!("{mode} {value}")
}

/// Quote a single POSIX shell argument, preserving apostrophes.
fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn net_config(discovery: Discovery) -> Result<NetConfig, Report> {
    let mut net = NetConfig::new(discovery);
    if let Some(relay) = env::var_os("JAMPARTY_RELAY").filter(|relay| !relay.is_empty()) {
        let relay = relay
            .to_str()
            .context("JAMPARTY_RELAY is not valid UTF-8")?;
        net.relay = Some(relay.parse::<RelayUrl>().context("JAMPARTY_RELAY")?);
    }
    Ok(net)
}

async fn share(
    source: Source,
    net: &ShareNet,
    delivery: Delivery,
    site: &str,
    detail: &str,
) -> Result<(), Report> {
    let discovery = if net.local {
        Discovery::Direct
    } else {
        Discovery::Public
    };
    let in_t3 = env::var_os("T3CODE_THREAD_ID").is_some();
    let (target, note) = target(delivery, &source, in_t3);
    let goes_to = match &target {
        Target::Recv => "jamparty recv".to_owned(),
        Target::Command(command) => format!(
            "the command {}",
            command
                .argv
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    let config = ShareConfig::new(
        source,
        net_config(discovery)?,
        target,
        runtime::default_dir()?,
    );
    let share = Share::start(config).await?;
    if discovery == Discovery::Public
        && time::timeout(ONLINE_TIMEOUT, share.endpoint().online())
            .await
            .is_err()
    {
        tracing::warn!("no relay reachable yet; joiners may not find this share");
    }
    // Startup records stay on stdout for agents reading the token and id;
    // ongoing diagnostics (including tracing) go to stderr.
    println!("jamparty: sharing {site}{detail}");
    println!("jamparty: token: {}", share.token());
    println!(
        "jamparty: warning: anyone with the token can see the site and send comments to the agent until this share stops"
    );
    println!("jamparty: share id: {}", share.id());
    println!("jamparty: comments go to {goes_to}");
    if let Some(note) = note {
        println!("jamparty: note: {note}");
    }
    println!("jamparty: agents: run jamparty agent for the guides");
    if discovery == Discovery::Direct {
        for addr in share.direct_addrs() {
            println!("jamparty: direct address: {addr}");
        }
    }
    shutdown_signal().await?;
    share.stop().await;
    Ok(())
}

/// Overrides of the settings file for this join only.
#[derive(Clone, Debug, Default, clap::Args)]
pub(crate) struct JoinSettings {
    /// Dismiss answers after this many seconds (default: 30; 0 disables;
    /// maximum 3600).
    #[arg(long)]
    answer_timeout: Option<AnswerTimeout>,
    /// Key for dismissing all answers (default: d).
    #[arg(long)]
    clear_answers_key: Option<KeyBinding>,
    /// Key for toggling comment mode (default: c).
    #[arg(long)]
    comment_mode_key: Option<KeyBinding>,
    /// Key for toggling your cursor (default: x).
    #[arg(long)]
    show_cursor_key: Option<KeyBinding>,
}

impl From<JoinSettings> for SettingsOverride {
    fn from(args: JoinSettings) -> Self {
        let JoinSettings {
            answer_timeout,
            clear_answers_key,
            comment_mode_key,
            show_cursor_key,
        } = args;
        Self {
            answer_timeout,
            clear_answers_key,
            comment_mode_key,
            show_cursor_key,
        }
    }
}

/// Whether `join` opens the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Browser,
    No,
}

async fn join(
    token: PartyToken,
    name: Option<JoinerName>,
    open: Open,
    share_addrs: Vec<SocketAddr>,
    port: Option<Port>,
    overrides: JoinSettings,
) -> Result<(), Report> {
    let discovery = if share_addrs.is_empty() {
        Discovery::Public
    } else {
        Discovery::Direct
    };
    // A key of this run's alone, never stored: shares cannot tell one join
    // from the next. Reconnects within the run keep it.
    let key = generate_key(&SystemRng);
    let config = JoinConfig {
        token,
        name,
        default_name: default_name(),
        settings: settings::default_path()?,
        overrides: overrides.into(),
        key,
        net: net_config(discovery)?,
        share_addrs,
        port,
        connect_wait: CONNECT_WAIT,
        admission_wait: ADMISSION_WAIT,
        proxy: ProxyLimits::JOIN,
    };
    eprintln!("jamparty: connecting to the share…");
    let joiner = tokio::select! {
        joiner = Joiner::start(config) => joiner?,
        signal = shutdown_signal() => return Ok(signal?),
    };
    if let Some(note) = joiner.version_note() {
        eprintln!("jamparty: {note}");
    }
    if let Some(fallback) = joiner.fallback() {
        eprintln!("jamparty: {fallback}");
    }
    let url = joiner.url();
    eprintln!("jamparty: open {url}");
    match open {
        Open::Browser => open_browser(&url),
        Open::No => {}
    }
    let mut snapshot = joiner.watch();
    let refused = tokio::select! {
        refused = snapshot.wait_for(|snapshot| snapshot.link.refused.is_some()) => {
            refused.ok().and_then(|snapshot| snapshot.link.refused)
        }
        signal = shutdown_signal() => {
            signal?;
            None
        }
    };
    let incompatible = joiner.incompatibility();
    joiner.stop().await;
    match (refused, incompatible) {
        (Some(_), Some(incompatible)) => Err(incompatible.into()),
        (Some(reason), None) => Err(Refused(reason).into()),
        (None, _) => Ok(()),
    }
}

fn default_name() -> JoinerName {
    let user = env::var("USER").unwrap_or_default();
    let host = unistd::gethostname()
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
    let spawned = Process::new(opener)
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
        result = ctrl_c() => {
            result?;
            if std::io::IsTerminal::is_terminal(&io::stderr()) {
                eprintln!();
            }
            Ok(())
        },
        _ = terminate.recv() => Ok(()),
        _ = hangup.recv() => Ok(()),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;

#[cfg(test)]
mod join_settings_tests {
    use clap::Parser;

    use crate::{
        Cli, Command, JoinSettings,
        core::settings::{Settings, SettingsOverride},
    };

    #[test]
    fn join_setting_options_parse_and_validate() {
        let cli = Cli::try_parse_from([
            "jamparty",
            "join",
            "-",
            "--answer-timeout",
            "3600",
            "--clear-answers-key",
            "q",
            "--comment-mode-key",
            "k",
            "--show-cursor-key",
            "F2",
        ])
        .unwrap();
        let Some(Command::Join { settings, .. }) = cli.command else {
            panic!("expected join")
        };
        let settings = SettingsOverride::from(settings)
            .apply(Settings::default())
            .unwrap();
        assert_eq!(u16::from(settings.answer_timeout), 3600);
        assert_eq!(
            serde_json::to_value(settings.keys).unwrap(),
            serde_json::json!({
                "commentMode": "k", "showCursor": "F2", "clearAnswers": "q"
            })
        );
        for value in ["-1", "3601", "1.5", "no"] {
            assert!(
                Cli::try_parse_from(["jamparty", "join", "-", "--answer-timeout", value]).is_err()
            );
        }
        assert!(
            Cli::try_parse_from(["jamparty", "join", "-", "--clear-answers-key", "Enter"]).is_err()
        );
    }

    #[test]
    fn join_overrides_keep_defaults_and_check_conflicts() {
        assert_eq!(
            SettingsOverride::from(JoinSettings::default())
                .apply(Settings::default())
                .unwrap(),
            Settings::default()
        );
        let cli =
            Cli::try_parse_from(["jamparty", "join", "-", "--clear-answers-key", "C"]).unwrap();
        let Some(Command::Join { settings, .. }) = cli.command else {
            panic!("expected join")
        };
        assert!(
            SettingsOverride::from(settings)
                .apply(Settings::default())
                .is_err()
        );
    }
}
