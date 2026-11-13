use std::{ffi::OsString, fs, time::Duration};

use clap::{CommandFactory, Parser};

use rootcause::{option_ext::OptionExt, prelude::ResultExt};

use crate::{
    Cli, Command, Delivery, Report, TokenArg,
    core::{
        ids::EndpointKey,
        rng::SeededRng,
        token::{PartySecret, PartyToken},
    },
    describe,
    protocol::agent::Addressee,
    share::delivery::{CommandTarget, Target},
    target,
};

fn token() -> String {
    PartyToken {
        share: EndpointKey([0xab; 32]),
        secret: PartySecret::generate(&SeededRng::new(7)),
    }
    .to_string()
}

#[test]
fn parses_the_commands() {
    let serve = Cli::try_parse_from(["jampot", "serve"]).unwrap();
    assert!(matches!(serve.command, Some(Command::Serve { .. })));
    let proxy = Cli::try_parse_from(["jampot", "proxy", "http://127.0.0.1:8000"]).unwrap();
    assert!(matches!(proxy.command, Some(Command::Proxy { .. })));
    let token = token();
    let join =
        Cli::try_parse_from(["jampot", "join", &token, "--name", "alice", "--no-open"]).unwrap();
    assert!(matches!(
        join.command,
        Some(Command::Join { no_open: true, .. })
    ));
    assert!(Cli::try_parse_from(["jampot", "join", "nonsense"]).is_err());
    // `-` reads the token from stdin, keeping it out of argv.
    let piped = Cli::try_parse_from(["jampot", "join", "-"]).unwrap();
    let Some(Command::Join { token: piped, .. }) = piped.command else {
        panic!("not join");
    };
    let read = piped
        .resolve(format!("{token}\nrest\n").as_bytes())
        .unwrap();
    assert_eq!(read.to_string(), token);
    let error = TokenArg::Stdin
        .resolve(b"nonsense\n".as_slice())
        .unwrap_err();
    assert!(error.to_string().contains("stdin"), "{error}");
    assert!(Cli::try_parse_from(["jampot", "join", &token, "--name", "a b"]).is_err());
    assert!(Cli::try_parse_from(["jampot", "http://127.0.0.1:8000"]).is_err());
    assert!(Cli::try_parse_from(["jampot"]).unwrap().command.is_none());
}

#[test]
fn agent_commands_parse_and_stay_out_of_help() {
    let id = "tzrjdfymwrg5sqai";
    let comment = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let send = Cli::try_parse_from([
        "jampot", "send", "hi", "--anchor", comment, "--share", id, "--to", "bob",
    ])
    .unwrap();
    let Some(Command::Send {
        message,
        anchor,
        to,
        which,
    }) = send.command
    else {
        panic!("not send");
    };
    assert_eq!(message.as_deref(), Some("hi"));
    assert_eq!(anchor.map(|id| id.to_string()).as_deref(), Some(comment));
    assert_eq!(which.share.map(|id| id.to_string()).as_deref(), Some(id));
    assert_eq!(to, Some(Addressee::Named("bob".parse().unwrap())));
    let everyone = Cli::try_parse_from(["jampot", "send", "hi", "--to", "all"]).unwrap();
    assert!(matches!(
        everyone.command,
        Some(Command::Send {
            to: Some(Addressee::Everyone),
            ..
        })
    ));
    assert!(Cli::try_parse_from(["jampot", "send", "hi", "--to", ""]).is_err());
    let recv = Cli::try_parse_from(["jampot", "recv", "--share", id]).unwrap();
    assert!(matches!(recv.command, Some(Command::Recv { which }) if which.share.is_some()));
    // Only the id: a token does not belong in argv.
    assert!(Cli::try_parse_from(["jampot", "recv", "--share", &token()]).is_err());
    assert!(Cli::try_parse_from(["jampot", "recv", "--share", "../x"]).is_err());
    assert!(Cli::try_parse_from(["jampot", "send", "hi", "--anchor", "1"]).is_err());
    assert!(Cli::try_parse_from(["jampot", "agent", "claude"]).is_ok());
    assert!(Cli::try_parse_from(["jampot", "agent", "vim"]).is_err());

    let help = Cli::command().render_long_help().to_string();
    for visible in ["serve", "proxy", "join"] {
        assert!(help.contains(visible), "{help}");
    }
    for hidden in ["agent", "recv", "send"] {
        assert!(!help.contains(&format!("  {hidden}")), "{help}");
    }
}

fn delivery(args: &[&str]) -> Delivery {
    let cli = Cli::try_parse_from(["jampot", "serve"].iter().chain(args)).unwrap();
    match cli.command {
        Some(Command::Serve { delivery, .. }) => delivery,
        other => panic!("not serve: {other:?}"),
    }
}

#[test]
fn delivery_options_choose_the_target() {
    let choose = |args: &[&str], env: Option<&str>| {
        target(delivery(args), "/srv/site", env.map(OsString::from))
    };
    let (recv, note) = choose(&[], None).unwrap();
    assert_eq!(recv, Target::Recv);
    assert!(note.is_none());
    let (_, note) = choose(&[], Some("thread-1")).unwrap();
    assert!(note.unwrap().contains("--t3-url"));

    let (command, note) = choose(&["site", "--", "agent", "--flag"], None).unwrap();
    assert_eq!(
        command,
        Target::Command(CommandTarget {
            argv: vec![OsString::from("agent"), OsString::from("--flag")],
            timeout: Duration::from_secs(60),
        })
    );
    assert!(note.is_none());
    let (command, _) = choose(&["--command-timeout", "5", "--", "agent"], None).unwrap();
    assert!(matches!(command, Target::Command(c) if c.timeout == Duration::from_secs(5)));
    let both = choose(
        &["--t3-url", "http://127.0.0.1:3773/mcp", "--", "agent"],
        None,
    );
    assert!(both.unwrap_err().to_string().contains("not both"));

    let t3 = [
        "--t3-url",
        "http://127.0.0.1:3773/mcp",
        "--t3-token-file",
        "/run/t3-token",
    ];
    let (from_env, _) = choose(&t3, Some("thread-1")).unwrap();
    let Target::T3(from_env) = from_env else {
        panic!("not t3: {from_env:?}");
    };
    assert_eq!(from_env.thread, "thread-1");
    assert_eq!(from_env.site, "/srv/site");
    assert_eq!(from_env.url.to_string(), "http://127.0.0.1:3773/mcp");
    let given = [&t3[..], &["--thread", "thread-2"]].concat();
    let (Target::T3(given), _) = choose(&given, Some("thread-1")).unwrap() else {
        panic!("not t3");
    };
    assert_eq!(given.thread, "thread-2");

    // Each missing piece names its option; nothing is probed.
    let missing = |args: &[&str], env: Option<&str>| choose(args, env).unwrap_err().to_string();
    assert!(missing(&t3, None).contains("--thread"));
    assert!(missing(&["--thread", "t"], None).contains("--t3-url"));
    assert!(
        missing(&["--t3-url", "http://127.0.0.1:3773/mcp"], Some("t")).contains("--t3-token-file")
    );
    let ftp = [
        "--t3-url",
        "ftp://t3",
        "--t3-token-file",
        "f",
        "--thread",
        "t",
    ];
    assert!(missing(&ftp, None).contains("http"));
    // Plain http only to this machine.
    let remote = |url: &str| {
        let args = ["--t3-url", url, "--t3-token-file", "f", "--thread", "t"];
        choose(&args, None).map(|_| ())
    };
    assert!(
        remote("http://t3/mcp")
            .unwrap_err()
            .to_string()
            .contains("https://")
    );
    assert!(remote("http://10.0.0.1/mcp").is_err());
    assert!(remote("http://localhost.evil.test/mcp").is_err());
    for local in [
        "https://t3/mcp",
        "http://localhost:3773/mcp",
        "http://127.0.0.2/mcp",
        "http://[::1]:3773/mcp",
    ] {
        assert!(remote(local).is_ok(), "{local}");
    }
}

#[test]
fn errors_read_as_written() {
    let missing: Result<(), Report> = None::<()>
        .context("the share stopped")
        .map_err(Report::from);
    assert_eq!(describe(&missing.unwrap_err()), "the share stopped");
    let io = fs::read("/nonexistent/jampot")
        .context("could not read the key")
        .map_err(Report::from);
    assert_eq!(
        describe(&io.unwrap_err()),
        "could not read the key: No such file or directory (os error 2)"
    );
}
