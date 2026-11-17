//! The share reads the hello (the line with the party secret) whole into
//! a fixed buffer: no longer than it, no escapes, and what follows it in
//! the same write is still read.

use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::io::BufReader;

use crate::{
    adapters::lines::{self, MAX_HELLO},
    protocol::control::{AckOutcome, Envelope, RefusedReason, ShareMessage},
    share::Share,
    tests::harness::{fixture_dir, raw_connection, share_files, within},
};

fn hello(secret: &str) -> Vec<u8> {
    format!("{{\"version\":1,\"message\":{{\"type\":\"hello\",\"secret\":\"{secret}\",\"name\":\"mallory\",\"jamparty\":\"{}\"}}}}\n", env!("CARGO_PKG_VERSION"))
        .into_bytes()
}

/// A raw control stream: what keeps it open, and its receiving side.
struct Raw {
    _endpoint: Endpoint,
    _connection: Connection,
    _send: SendStream,
    recv: BufReader<RecvStream>,
}

/// Opens a control stream on `share`, writes `first` in one go and returns
/// the share's first reply.
async fn first_reply(share: &Share, first: &[u8]) -> (ShareMessage, Raw) {
    let (endpoint, connection) = raw_connection(share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    send.write_all(first).await.unwrap();
    let mut recv = BufReader::new(recv);
    let reply = within(
        "a reply",
        lines::read::<Envelope<ShareMessage>, _>(&mut recv),
    )
    .await
    .unwrap()
    .unwrap()
    .message;
    let raw = Raw {
        _endpoint: endpoint,
        _connection: connection,
        _send: send,
        recv,
    };
    (reply, raw)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hello_is_read_whole_into_a_fixed_buffer() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let secret = share.token().secret.to_string();

    // Padded past the hello's buffer: refused, though the secret is right.
    let padded = String::from_utf8(hello(&secret))
        .unwrap()
        .replace(",\"name\"", &format!("{},\"name\"", " ".repeat(MAX_HELLO)));
    let (reply, _) = first_reply(&share, padded.as_bytes()).await;
    assert_eq!(
        reply,
        ShareMessage::Refused {
            reason: RefusedReason::BadHello,
            jamparty: None,
        }
    );

    // An escaped secret would be unescaped into serde_json's own buffer:
    // refused, though it means the right secret.
    let (first, rest) = secret.split_at(1);
    let escaped = hello(&format!("\\u00{:02x}{rest}", first.bytes().next().unwrap()));
    let (reply, _) = first_reply(&share, &escaped).await;
    assert_eq!(
        reply,
        ShareMessage::Refused {
            reason: RefusedReason::BadHello,
            jamparty: None,
        }
    );

    // A hello and a comment in one write: both are read.
    let mut both = hello(&secret);
    both.extend_from_slice(
        br#"{"version":1,"message":{"type":"comment","ref":"r1","draft":{"page":"/","selector":"","comment":"hi"}}}"#,
    );
    both.push(b'\n');
    let (reply, mut raw) = first_reply(&share, &both).await;
    assert!(matches!(reply, ShareMessage::Admitted { .. }), "{reply:?}");
    let acked = within("the ack", async {
        loop {
            let message = lines::read::<Envelope<ShareMessage>, _>(&mut raw.recv)
                .await
                .unwrap()
                .unwrap()
                .message;
            if let ShareMessage::CommentAck { outcome, .. } = message {
                return outcome;
            }
        }
    })
    .await;
    assert!(matches!(acked, AckOutcome::Queued { .. }), "{acked:?}");
    assert_eq!(share.queued().len(), 1);
}
