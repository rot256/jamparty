use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode, client::conn::http1, header};
use hyper_util::rt::TokioIo;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde_json::{Value, json};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpStream, UnixStream},
};

use crate::{
    adapters::lines,
    agent,
    protocol::{
        control::{Envelope, ShareMessage},
        resource::{MAX_FILE_BYTES, ResourceId, UPLOAD_PATH, Uploaded},
    },
    share::Share,
    tests::harness::{
        DRAFT, commented, fetch, fixture_dir, join, local, post_comment, raw_connection,
        share_files, within,
    },
};

#[tokio::test(flavor = "multi_thread")]
async fn attachment_reaches_recv_and_fetch_across_shares() {
    let site = fixture_dir();
    let share = share_files(&site).await;
    let runtime = share.socket_path().parent().unwrap();
    let mut other_config = crate::tests::harness::config(
        crate::share::Source::Files(site.path().to_path_buf()),
        crate::share::delivery::Target::Recv,
    );
    other_config.runtime_dir = runtime.to_path_buf();
    let _other = crate::share::Share::start(other_config).await.unwrap();
    let joiner = join(&share, "alice").await;

    // Larger than a control line, with arbitrary binary content.
    let data = Bytes::from(vec![0xfe; 300_000]);
    let upload = |body| {
        local(&joiner, "POST", UPLOAD_PATH)
            .header(header::ORIGIN, joiner.origin().origin())
            .body(Full::new(body))
            .unwrap()
    };
    let mut oversized = TcpStream::connect(joiner.local_addr()).await.unwrap();
    let head = format!(
        "POST {UPLOAD_PATH} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nContent-Length: {}\r\n\r\n",
        joiner.origin().host(),
        joiner.origin().origin(),
        MAX_FILE_BYTES + 1
    );
    // Refuse at the header without racing a body write against the close.
    oversized.write_all(head.as_bytes()).await.unwrap();
    let mut status = String::new();
    within(
        "oversized upload refusal",
        BufReader::new(oversized).read_line(&mut status),
    )
    .await
    .unwrap();
    assert!(status.starts_with("HTTP/1.1 413 "), "{status:?}");
    let refused = fetch(
        &joiner,
        local(&joiner, "POST", UPLOAD_PATH)
            .header(header::ORIGIN, "http://other.localhost")
            .body(Full::new(data.clone()))
            .unwrap(),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let reply = fetch(&joiner, upload(data.clone())).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let uploaded: Uploaded = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(uploaded.id, ResourceId::digest(&data));

    let mut draft: Value = serde_json::from_str(DRAFT).unwrap();
    draft["comment"] = "".into();
    let missing = ResourceId::digest(b"not uploaded");
    draft["attachments"] = json!([{"type":"file","file":missing,"name":"other"}]);
    let refused = post_comment(&joiner, &draft.to_string()).await;
    assert_eq!(
        refused["refused"]["attachmentUnavailable"], true,
        "{refused}"
    );
    assert!(share.queued().is_empty());
    draft["attachments"] = json!([{"type":"file","file":uploaded.id,"name":"/a\\b\n.bin"}]);
    let outcome = post_comment(&joiner, &draft.to_string()).await;
    assert!(commented(&outcome).is_some(), "{outcome}");

    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    let mut out = Vec::new();
    assert_eq!(
        within("recv", agent::recv(stream, &mut out)).await.unwrap(),
        1
    );
    let delivered: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(delivered["comment"], "");
    assert_eq!(
        delivered["attachments"],
        json!([{"type":"file","file":uploaded.id,"name":"ab.bin"}])
    );

    let resource = within("fetch", agent::fetch::resource(runtime, uploaded.id))
        .await
        .unwrap();
    let mut written = Vec::new();
    agent::fetch::write_verified(uploaded.id, &resource, Path::new("-"), &mut written).unwrap();
    assert_eq!(written, data);
    let error = agent::fetch::resource(runtime, missing).await.unwrap_err();
    assert!(error.to_string().contains("not found"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn share_hashes_direct_uploads_and_caps_chunked_requests() {
    let site = fixture_dir();
    let share = share_files(&site).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let _control = admit(&share, &connection).await;

    let (send, recv_upload) = connection.open_bi().await.unwrap();
    let (mut sender, driver) = http1::handshake(TokioIo::new(io::join(recv_upload, send)))
        .await
        .unwrap();
    tokio::spawn(driver);
    let wrong = ResourceId::digest(b"anything other than the uploaded file");
    let bytes = b"the uploaded file";
    let upload = Request::post(UPLOAD_PATH)
        .header(header::HOST, "jamparty")
        .header("x-attachment-id", wrong.to_string())
        .body(Full::new(Bytes::from_static(bytes)))
        .unwrap();
    let response = within("upload response", sender.send_request(upload))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = within("upload body", response.into_body().collect())
        .await
        .unwrap();
    let uploaded: Uploaded = serde_json::from_slice(&body.to_bytes()).unwrap();
    assert_eq!(uploaded.id, ResourceId::digest(bytes));
    assert!(share.hub().resource(wrong).is_none());

    // Bypass the local API. Withhold the terminating chunk and keep the
    // write half open: the share must reply as soon as the body crosses 8 MiB.
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let mut recv = BufReader::new(recv);
    let head = format!(
        "POST {UPLOAD_PATH} HTTP/1.1\r\nHost: jamparty\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        MAX_FILE_BYTES + 1
    );
    let write = async {
        send.write_all(head.as_bytes()).await.unwrap();
        send.write_all(&vec![0; MAX_FILE_BYTES + 1]).await
    };
    let read = async {
        let mut status = String::new();
        recv.read_line(&mut status).await.unwrap();
        status
    };
    let (_, status) = within("chunked upload refusal", async {
        tokio::join!(write, read)
    })
    .await;
    assert!(status.starts_with("HTTP/1.1 413 "), "{status:?}");
}

async fn admit(share: &Share, connection: &Connection) -> (SendStream, BufReader<RecvStream>) {
    let (mut control, recv) = connection.open_bi().await.unwrap();
    let hello = json!({"version":1, "message": {
        "type":"hello", "secret":share.token().secret.to_string(),
        "name":"alice","jamparty":env!("CARGO_PKG_VERSION"),
    }});
    control
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .unwrap();
    let mut recv = BufReader::new(recv);
    let reply = within(
        "admission",
        lines::read::<Envelope<ShareMessage>, _>(&mut recv),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(reply.message, ShareMessage::Admitted { .. }));

    (control, recv)
}

#[tokio::test(flavor = "multi_thread")]
async fn one_connection_cannot_use_another_upload_slot() {
    let site = fixture_dir();
    let share = share_files(&site).await;
    let (_first_endpoint, first) = raw_connection(&share).await;
    let (_second_endpoint, second) = raw_connection(&share).await;
    let _first_control = admit(&share, &first).await;
    let _second_control = admit(&share, &second).await;

    let (mut slow_send, slow_recv) = first.open_bi().await.unwrap();
    let mut slow_recv = BufReader::new(slow_recv);
    slow_send.write_all(format!(
        "POST {UPLOAD_PATH} HTTP/1.1\r\nHost: jamparty\r\nContent-Length: 1\r\nExpect: 100-continue\r\n\r\n"
    ).as_bytes()).await.unwrap();
    let mut status = String::new();
    within(
        "the first upload to hold its slot",
        slow_recv.read_line(&mut status),
    )
    .await
    .unwrap();
    assert!(status.starts_with("HTTP/1.1 100 "), "{status:?}");

    async fn upload(connection: &Connection) -> StatusCode {
        let (send, recv) = connection.open_bi().await.unwrap();
        let (mut sender, driver) = http1::handshake(TokioIo::new(io::join(recv, send)))
            .await
            .unwrap();
        tokio::spawn(driver);
        let request = Request::post(UPLOAD_PATH)
            .header(header::HOST, "jamparty")
            .body(Full::<Bytes>::default())
            .unwrap();
        within("upload response", sender.send_request(request))
            .await
            .unwrap()
            .status()
    }
    assert_eq!(upload(&first).await, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(upload(&second).await, StatusCode::CREATED);
}
