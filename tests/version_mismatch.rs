//! `jamparty join` against a share of another release line, as a person runs
//! it: it exits with an error on stderr naming both versions and what to
//! install. The share here is a stand-in that greets with an incompatible release line.

use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};

use iroh::{Endpoint, RelayMode, SecretKey, endpoint::presets};

/// Stable transport framing lets incompatible release lines explain themselves.
const ALPN: &[u8] = b"jamparty/1";

#[tokio::test(flavor = "multi_thread")]
async fn join_names_both_versions_and_exits() {
    let key = SecretKey::from_bytes(&[7; 32]);
    let share = key.public();
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let addr = endpoint.bound_sockets().into_iter().next().unwrap();
    let accepting = endpoint.clone();
    tokio::spawn(async move {
        let mut kept = Vec::new();
        while let Some(incoming) = accepting.accept().await {
            let Ok(connection) = incoming.await else {
                continue;
            };
            let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                continue;
            };
            let mut byte = [0];
            while recv.read_exact(&mut byte).await.is_ok() && byte != *b"\n" {}
            let line =
                b"{\"version\":2,\"message\":{\"type\":\"welcome\",\"jamparty\":\"999.0.0\"}}\n";
            let _ = send.write_all(line).await;
            kept.push((connection, send, recv));
        }
    });
    // The token: the share's key and a party secret, in base58.
    let mut token = share.as_bytes().to_vec();
    token.extend_from_slice(&[1; 16]);
    let mut text = [0; 128];
    let written = bs58::encode(token).onto(text.as_mut_slice()).unwrap();
    let token = std::str::from_utf8(text.get(..written).unwrap())
        .unwrap()
        .to_owned();

    let home = tempfile::tempdir().unwrap();
    let mut join = Command::new(env!("CARGO_BIN_EXE_jamparty"))
        .args(["join", "-", "--no-open", "--share-addr", &addr.to_string()])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_RUNTIME_DIR", home.path().join("run"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(join.stdin.take().unwrap(), "{token}").unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || join.wait_with_output()),
    )
    .await
    .expect("jamparty join exits")
    .unwrap()
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ours = env!("CARGO_PKG_VERSION");
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "jamparty: error: the share runs jamparty 999.0.0; this is jamparty {ours}. Install the share's version: cargo install --locked jamparty@999.0.0"
        )),
        "{stderr}"
    );
    endpoint.close().await;
}
