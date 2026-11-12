use std::ffi::OsString;

use clap::{CommandFactory, Parser};

use crate::{Cli, Command, Delivery, agent::guides, share::delivery::Target, target};

fn token() -> String {
    format!("{}:{}", "ab".repeat(32), "a".repeat(26))
}

#[test]
fn parses_the_commands() {
    let serve = Cli::try_parse_from(["jamjam", "serve"]).unwrap();
    assert!(matches!(serve.command, Some(Command::Serve { .. })));
    let proxy = Cli::try_parse_from(["jamjam", "proxy", "http://127.0.0.1:8000"]).unwrap();
    assert!(matches!(proxy.command, Some(Command::Proxy { .. })));
    let token = token();
    let join =
        Cli::try_parse_from(["jamjam", "join", &token, "--name", "alice", "--no-open"]).unwrap();
    assert!(matches!(
        join.command,
        Some(Command::Join { no_open: true, .. })
    ));
    assert!(Cli::try_parse_from(["jamjam", "join", "nonsense"]).is_err());
    assert!(Cli::try_parse_from(["jamjam", "join", &token, "--name", "a b"]).is_err());
    assert!(Cli::try_parse_from(["jamjam", "http://127.0.0.1:8000"]).is_err());
    assert!(Cli::try_parse_from(["jamjam"]).unwrap().command.is_none());
}

#[test]
fn agent_commands_parse_and_stay_out_of_help() {
    let id = "mzuhvlpymk6xo3ep";
    let comment = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let send =
        Cli::try_parse_from(["jamjam", "send", "hi", "--anchor", comment, "--share", id]).unwrap();
    let Some(Command::Send {
        message,
        anchor,
        which,
    }) = send.command
    else {
        panic!("not send");
    };
    assert_eq!(message.as_deref(), Some("hi"));
    assert_eq!(anchor.map(|id| id.to_string()).as_deref(), Some(comment));
    assert_eq!(which.share.map(|id| id.to_string()).as_deref(), Some(id));
    // A token names its share too.
    let recv = Cli::try_parse_from(["jamjam", "recv", "--share", &token()]).unwrap();
    assert!(matches!(recv.command, Some(Command::Recv { which }) if which.share.is_some()));
    assert!(Cli::try_parse_from(["jamjam", "recv", "--share", "../x"]).is_err());
    assert!(Cli::try_parse_from(["jamjam", "send", "hi", "--anchor", "1"]).is_err());
    assert!(Cli::try_parse_from(["jamjam", "agent", "claude"]).is_ok());
    assert!(Cli::try_parse_from(["jamjam", "agent", "vim"]).is_err());

    let help = Cli::command().render_long_help().to_string();
    for visible in ["serve", "proxy", "join"] {
        assert!(help.contains(visible), "{help}");
    }
    for hidden in ["agent", "recv", "send"] {
        assert!(!help.contains(&format!("  {hidden}")), "{help}");
    }
}

fn delivery(args: &[&str]) -> Delivery {
    let cli = Cli::try_parse_from(["jamjam", "serve"].iter().chain(args)).unwrap();
    match cli.command {
        Some(Command::Serve { delivery, .. }) => delivery,
        other => panic!("not serve: {other:?}"),
    }
}

#[test]
fn delivery_options_choose_the_target() {
    let (recv, _) = target(delivery(&[])).unwrap();
    assert_eq!(recv, Target::Recv);
    let (command, note) = target(delivery(&["site", "--", "agent", "--flag"])).unwrap();
    assert_eq!(
        command,
        Target::Command(vec![OsString::from("agent"), OsString::from("--flag")])
    );
    assert!(note.is_none());
    let both = target(delivery(&["--t3-url", "http://t3", "--", "agent"]));
    assert!(both.unwrap_err().to_string().contains("not both"));
    assert!(target(delivery(&["--thread", "t"])).is_err());
}

#[test]
fn bare_jamjam_prints_the_overview_in_the_design() {
    let design = include_str!("../DESIGN.md");
    let start = design
        .find("Bare `jamjam` prints, always the same:")
        .unwrap();
    let block = design.get(start..).unwrap();
    let open = block.find("```\n").unwrap() + 4;
    let close = block.get(open..).unwrap().find("```").unwrap();
    assert_eq!(block.get(open..open + close).unwrap(), guides::OVERVIEW);
}
