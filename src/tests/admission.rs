//! Admission: nothing is served before the right hello; the joiner only
//! accepts the share in its token.

use std::{fs, future::Future, time::Duration};

use iroh::endpoint::{Connection, ConnectionError, VarInt};
use tokio::{io::BufReader, time};

use crate::{
    adapters::{
        endpoint::{Accept, bind, connect, generate_key},
        lines,
    },
    core::{
        ids::EndpointKey,
        rng::SystemRng,
        token::{PartySecret, PartyToken},
    },
    protocol::{
        control::{Envelope, RefusedReason, ShareMessage, SiteInfo},
        presence::Support,
    },
    share::{REFUSED_CODE, Source, delivery::Target},
    tests::harness::{
        config, connected, fixture_dir, get, joiner, net, raw_connection, share_files, share_proxy,
        start, within,
    },
};

const HTTP_REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";

/// Opens a request stream and returns whatever the share answers on it.
async fn try_request(connection: &Connection) -> Vec<u8> {
    // The request says `Connection: close`, so the share ends the stream
    // after answering (hyper treats a half-closed stream as gone).
    let Ok((mut send, mut recv)) = connection.open_bi().await else {
        return Vec::new();
    };
    if send.write_all(HTTP_REQUEST).await.is_err() {
        return Vec::new();
    }
    // Keep whatever arrives, even if the connection closes afterwards.
    let mut answered = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(Some(read)) = within("the request stream", recv.read(&mut buffer)).await {
        answered.extend_from_slice(buffer.get(..read).unwrap_or_default());
    }
    answered
}

/// Sends `first` as the control stream's first bytes, plus a request
/// stream; returns the share's control reply and the request's answer.
async fn first_line(first: &[u8]) -> (Option<ShareMessage>, Vec<u8>, ConnectionError) {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    send.write_all(first).await.unwrap();
    let answered = try_request(&connection).await;
    let mut recv = BufReader::new(recv);
    let reply = lines::read::<Envelope<ShareMessage>, _>(&mut recv)
        .await
        .ok()
        .flatten()
        .map(|envelope| envelope.message);
    let closed = within("the close", connection.closed()).await;
    (reply, answered, closed)
}

fn assert_refused(reply: Option<ShareMessage>, closed: &ConnectionError, reason: RefusedReason) {
    assert_eq!(reply, Some(ShareMessage::Refused { reason }));
    let ConnectionError::ApplicationClosed(close) = closed else {
        panic!("expected an application close, got {closed}");
    };
    assert_eq!(close.error_code, VarInt::from_u32(REFUSED_CODE));
    assert_eq!(RefusedReason::try_from(close.reason.as_ref()), Ok(reason));
}

fn hello(secret: &str) -> Vec<u8> {
    format!("{{\"version\":1,\"message\":{{\"type\":\"hello\",\"secret\":\"{secret}\",\"name\":\"mallory\"}}}}\n")
        .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_secret_is_refused_before_anything_is_served() {
    let wrong = PartySecret::generate(&SystemRng).to_string();
    let (reply, answered, closed) = first_line(&hello(&wrong)).await;
    assert_refused(reply, &closed, RefusedReason::WrongSecret);
    assert!(answered.is_empty(), "served before admission: {answered:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_secret_or_http_first_is_a_bad_hello() {
    let no_secret = b"{\"version\":1,\"message\":{\"type\":\"hello\",\"name\":\"mallory\"}}\n";
    for first in [no_secret.as_slice(), HTTP_REQUEST] {
        let (reply, answered, closed) = first_line(first).await;
        assert_refused(reply, &closed, RefusedReason::BadHello);
        assert!(answered.is_empty(), "served before admission: {answered:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn silence_is_refused_after_the_hello_timeout() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, _recv) = connection.open_bi().await.unwrap();
    send.write_all(b"{").await.unwrap();
    let closed = within("the close", connection.closed()).await;
    let ConnectionError::ApplicationClosed(close) = closed else {
        panic!("expected an application close, got {closed}");
    };
    assert_eq!(close.reason.as_ref(), b"bad_hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn right_secret_is_admitted_then_served() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    send.write_all(&hello(&share.token().secret.to_string()))
        .await
        .unwrap();
    let mut recv = BufReader::new(recv);
    let admitted = lines::read::<Envelope<ShareMessage>, _>(&mut recv)
        .await
        .unwrap()
        .unwrap();
    let ShareMessage::Admitted {
        you,
        undelivered,
        presence,
        site,
    } = admitted.message
    else {
        panic!("expected admission, got {admitted:?}");
    };
    assert_eq!(you.name.to_string(), "mallory");
    assert!(undelivered.is_empty());
    assert_eq!(presence, Support::Supported);
    assert_eq!(site, Some(SiteInfo::Files));
    let answered = String::from_utf8(try_request(&connection).await).unwrap();
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    assert!(answered.contains("Fixture"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_from_another_run_is_refused() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let earlier = share_files(&dir).await;
    let stale = PartyToken {
        share: share.token().share,
        secret: earlier.token().secret.clone(),
    };
    let joiner = joiner(stale, share.direct_addrs(), "late").await;
    let mut view = joiner.watch();
    let view = within("the refusal", view.wait_for(|view| view.refused.is_some()))
        .await
        .unwrap()
        .clone();
    assert_eq!(view.refused, Some(RefusedReason::WrongSecret));
    assert!(!view.connected);
    assert_eq!(get(&joiner, "/").await.status, 502);
    assert!(share.presence().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_only_accepts_the_share_in_its_token() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let endpoint = bind(&net(), generate_key(&SystemRng), Accept::Nothing)
        .await
        .unwrap();
    let impostor = EndpointKey::from(generate_key(&SystemRng).public());
    let result = within(
        "the failed handshake",
        connect(&endpoint, impostor, &share.direct_addrs()),
    )
    .await;
    assert!(result.is_err(), "connected to a share with another key");

    // The real key at the same address works.
    let token = share.token().clone();
    let joiner = joiner(token, share.direct_addrs(), "alice").await;
    connected(&joiner).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_hello_ends_the_session() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let line = hello(&share.token().secret.to_string());
    send.write_all(&line).await.unwrap();
    let mut recv = BufReader::new(recv);
    let _admitted = lines::read::<Envelope<ShareMessage>, _>(&mut recv).await;
    send.write_all(&line).await.unwrap();
    within("the close", connection.closed()).await;
}

/// Whether `future` is still pending after a while.
async fn stays_pending<T>(future: impl Future<Output = T>) -> bool {
    time::timeout(Duration::from_millis(500), future)
        .await
        .is_err()
}

#[tokio::test(flavor = "multi_thread")]
async fn before_admission_only_the_control_stream_opens() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    send.write_all(b"{").await.unwrap();
    assert!(stays_pending(connection.open_bi()).await, "a second stream");
    assert!(
        stays_pending(connection.open_uni()).await,
        "a unidirectional stream"
    );

    // Admission lifts the limit: several request streams at once.
    send.write_all(&hello(&share.token().secret.to_string()).split_off(1))
        .await
        .unwrap();
    let mut recv = BufReader::new(recv);
    let admitted = lines::read::<Envelope<ShareMessage>, _>(&mut recv)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(admitted.message, ShareMessage::Admitted { .. }));
    let mut open = Vec::new();
    for _ in 0..8 {
        open.push(
            within("a request stream", connection.open_bi())
                .await
                .unwrap(),
        );
    }
    assert!(stays_pending(connection.open_uni()).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_deadline_covers_the_whole_admission() {
    // The harness allows 2 s: the stream opens after 1.2 s and the hello
    // completes after 2.4 s, each within 2 s of the step before.
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    time::sleep(Duration::from_millis(1200)).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let line = hello(&share.token().secret.to_string());
    let (start, rest) = line.split_at(1);
    send.write_all(start).await.unwrap();
    time::sleep(Duration::from_millis(1200)).await;
    let _ = send.write_all(rest).await;
    let reply = lines::read::<Envelope<ShareMessage>, _>(&mut BufReader::new(recv))
        .await
        .ok()
        .flatten()
        .map(|envelope| envelope.message);
    let closed = within("the close", connection.closed()).await;
    assert_refused(reply, &closed, RefusedReason::BadHello);
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_waiting_for_admission_are_capped() {
    let dir = fixture_dir();
    let root = fs::canonicalize(dir.path()).unwrap();
    let mut config = config(Source::Files(root), Target::Recv);
    config.max_unadmitted = 1;
    let share = start(config).await;
    let (_first_endpoint, first) = raw_connection(&share).await;
    let endpoint = bind(&net(), generate_key(&SystemRng), Accept::Nothing)
        .await
        .unwrap();
    let refused = within(
        "the refused handshake",
        connect(&endpoint, share.token().share, &share.direct_addrs()),
    )
    .await;
    assert!(refused.is_err(), "a second unadmitted connection");

    // Once the first is admitted, the next may wait its turn.
    let (mut send, recv) = first.open_bi().await.unwrap();
    send.write_all(&hello(&share.token().secret.to_string()))
        .await
        .unwrap();
    let admitted = lines::read::<Envelope<ShareMessage>, _>(&mut BufReader::new(recv))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(admitted.message, ShareMessage::Admitted { .. }));
    let joiner = joiner(share.token().clone(), share.direct_addrs(), "alice").await;
    connected(&joiner).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proxy_share_announces_its_dev_server_port() {
    // Nothing needs to listen there: the share only names the port.
    let upstream = "127.0.0.1:1313".parse().unwrap();
    let share = share_proxy(upstream).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    send.write_all(&hello(&share.token().secret.to_string()))
        .await
        .unwrap();
    let admitted = lines::read::<Envelope<ShareMessage>, _>(&mut BufReader::new(recv))
        .await
        .unwrap()
        .unwrap();
    let ShareMessage::Admitted { site, .. } = admitted.message else {
        panic!("expected admission, got {admitted:?}");
    };
    assert_eq!(
        site,
        Some(SiteInfo::Proxy {
            port: 1313.try_into().unwrap()
        })
    );
}
