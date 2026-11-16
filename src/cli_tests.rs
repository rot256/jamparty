use std::{ffi::OsString, fs, time::Duration};

use clap::{CommandFactory, Parser};

use rootcause::{option_ext::OptionExt, prelude::ResultExt};

use crate::{
    Cli, Command, Delivery, Report, Source, TokenArg,
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
    let serve = Cli::try_parse_from(["jamparty", "serve"]).unwrap();
    assert!(matches!(serve.command, Some(Command::Serve { .. })));
    let proxy = Cli::try_parse_from(["jamparty", "proxy", "http://127.0.0.1:8000"]).unwrap();
    assert!(matches!(proxy.command, Some(Command::Proxy { .. })));
    let token = token();
    let join =
        Cli::try_parse_from(["jamparty", "join", &token, "--name", "alice", "--no-open"]).unwrap();
    assert!(matches!(
        join.command,
        Some(Command::Join { no_open: true, .. })
    ));
    assert!(Cli::try_parse_from(["jamparty", "join", "nonsense"]).is_err());
    // `-` reads the token from stdin, keeping it out of argv.
    let piped = Cli::try_parse_from(["jamparty", "join", "-"]).unwrap();
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
    assert!(Cli::try_parse_from(["jamparty", "join", &token, "--name", "a b"]).is_err());
    assert!(Cli::try_parse_from(["jamparty", "http://127.0.0.1:8000"]).is_err());
    assert!(Cli::try_parse_from(["jamparty"]).unwrap().command.is_none());
}

#[test]
fn agent_commands_parse_and_stay_out_of_help() {
    let id = "tzrjdfymwrg5sqai";
    let comment = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let send = Cli::try_parse_from([
        "jamparty", "send", "hi", "--anchor", comment, "--share", id, "--to", "bob",
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
    let everyone = Cli::try_parse_from(["jamparty", "send", "hi", "--to", "all"]).unwrap();
    assert!(matches!(
        everyone.command,
        Some(Command::Send {
            to: Some(Addressee::Everyone),
            ..
        })
    ));
    assert!(Cli::try_parse_from(["jamparty", "send", "hi", "--to", ""]).is_err());
    let recv = Cli::try_parse_from(["jamparty", "recv", "--share", id]).unwrap();
    assert!(matches!(recv.command, Some(Command::Recv { which }) if which.share.is_some()));
    // Only the id: a token does not belong in argv.
    assert!(Cli::try_parse_from(["jamparty", "recv", "--share", &token()]).is_err());
    assert!(Cli::try_parse_from(["jamparty", "recv", "--share", "../x"]).is_err());
    assert!(Cli::try_parse_from(["jamparty", "send", "hi", "--anchor", "1"]).is_err());
    assert!(Cli::try_parse_from(["jamparty", "agent", "claude"]).is_ok());
    assert!(Cli::try_parse_from(["jamparty", "agent", "vim"]).is_err());

    let help = Cli::command().render_long_help().to_string();
    for visible in ["serve", "proxy", "join"] {
        assert!(help.contains(visible), "{help}");
    }
    for hidden in ["agent", "recv", "send"] {
        assert!(!help.contains(&format!("  {hidden}")), "{help}");
    }
}

fn delivery(args: &[&str]) -> Delivery {
    let cli = Cli::try_parse_from(["jamparty", "serve"].iter().chain(args)).unwrap();
    match cli.command {
        Some(Command::Serve { delivery, .. }) => delivery,
        other => panic!("not serve: {other:?}"),
    }
}

#[test]
fn delivery_options_choose_the_target() {
    let choose = |args: &[&str], in_t3: bool| {
        target(delivery(args), &Source::Files("/srv/site".into()), in_t3)
    };
    let (recv, note) = choose(&[], false);
    assert_eq!(recv, Target::Recv);
    assert!(note.is_none());
    let (_, note) = choose(&[], true);
    assert_eq!(
        note.unwrap(),
        r#"inside T3, deliver to this thread with: jamparty serve /srv/site -- t3 thread send "$T3CODE_THREAD_ID" -"#
    );

    let (command, note) = choose(&["site", "--", "agent", "--flag"], true);
    assert_eq!(
        command,
        Target::Command(CommandTarget {
            argv: vec![OsString::from("agent"), OsString::from("--flag")],
            timeout: Duration::from_secs(60),
        })
    );
    assert!(note.is_none());
    for (given, secs) in [("5", 5), ("90s", 90), ("2m", 120), ("1h", 3600)] {
        let (command, _) = choose(&["--command-timeout", given, "--", "agent"], false);
        assert!(
            matches!(&command, Target::Command(c) if c.timeout == Duration::from_secs(secs)),
            "{given}: {command:?}"
        );
    }
    for bad in ["0", "", "5d", "-1", "1.5"] {
        let args = ["jamparty", "serve", "--command-timeout", bad, "--", "agent"];
        assert!(Cli::try_parse_from(args).is_err(), "{bad}");
    }
    for option in ["--t3-url", "--t3-token-file", "--thread"] {
        assert!(Cli::try_parse_from(["jamparty", "serve", option, "value"]).is_err());
    }
}

#[test]
fn t3_startup_note_pastes_exactly_or_not_at_all() {
    use std::{os::unix::ffi::OsStrExt, path::PathBuf};

    let files = |path: &[u8]| Source::Files(PathBuf::from(std::ffi::OsStr::from_bytes(path)));
    for (source, shown) in [
        (
            files(b"/srv/it's a site"),
            r#"serve '/srv/it'"'"'s a site'"#,
        ),
        (
            Source::Proxy("http://127.0.0.1:8000/a b".parse().unwrap()),
            "proxy http://127.0.0.1:8000/a%20b/",
        ),
        // Not UTF-8: pasting a lossy path would share another directory.
        (files(b"/srv/\xffsite"), "serve <dir>"),
        // Control characters would drive the terminal it is printed to.
        (files(b"/srv/\x1b[2J\rsite"), "serve <dir>"),
        (files("/srv/\u{202e}site".as_bytes()), "serve <dir>"),
    ] {
        let (_, note) = target(delivery(&[]), &source, true);
        assert_eq!(
            note.unwrap(),
            format!(
                r#"inside T3, deliver to this thread with: jamparty {shown} -- t3 thread send "$T3CODE_THREAD_ID" -"#
            )
        );
    }
}

#[test]
fn errors_read_as_written() {
    let missing: Result<(), Report> = None::<()>
        .context("the share stopped")
        .map_err(Report::from);
    assert_eq!(describe(&missing.unwrap_err()), "the share stopped");
    let io = fs::read("/nonexistent/jamparty")
        .context("could not read the key")
        .map_err(Report::from);
    assert_eq!(
        describe(&io.unwrap_err()),
        "could not read the key: No such file or directory (os error 2)"
    );
}
