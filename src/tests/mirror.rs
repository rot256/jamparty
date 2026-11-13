//! The joiner serves a proxied site on the dev server's port when it can,
//! so the dev server's own scripts that dial that port reach it.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};

use crate::{
    core::port::Port,
    join::Fallback,
    tests::harness::{fixture_dir, joiner_on, share_files, share_proxy},
};

/// A port free on 127.0.0.1 and [::1] (as far as can be told).
fn free_port() -> Port {
    let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = v4.local_addr().unwrap().port();
    // Taken on [::1] already: try another.
    if TcpListener::bind((Ipv6Addr::LOCALHOST, port)).is_err()
        && TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_ok()
    {
        return free_port();
    }
    port.try_into().unwrap()
}

/// A dev server URL on `port` of 127.0.0.2: nothing needs to listen there,
/// the share only announces the port.
fn dev_server(port: Port) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::new(127, 0, 0, 2), u16::from(port)))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_serves_on_the_dev_servers_port() {
    let port = free_port();
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    assert_eq!(joiner.local_addr().port(), u16::from(port));
    assert!(joiner.fallback().is_none());
    assert!(
        joiner.url().ends_with(&format!(".localhost:{port}/")),
        "{}",
        joiner.url()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_port_falls_back_to_a_free_one() {
    let port = free_port();
    let _taken = TcpListener::bind((Ipv4Addr::LOCALHOST, u16::from(port))).unwrap();
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    let serving = joiner.local_addr().port();
    assert_ne!(serving, u16::from(port));
    let Some(Fallback { wanted, .. }) = joiner.fallback() else {
        panic!("no fallback");
    };
    assert_eq!(*wanted, port);
    let note = joiner.fallback().unwrap().to_string();
    assert!(
        note.starts_with(&format!(
            "port {port} is in use here; serving on {serving}."
        )),
        "{note}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_taken_on_the_ipv6_side_falls_back_too() {
    let port = free_port();
    let Ok(_taken) = TcpListener::bind((Ipv6Addr::LOCALHOST, u16::from(port))) else {
        return; // No IPv6 loopback here: nobody can take that side.
    };
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    assert_ne!(joiner.local_addr().port(), u16::from(port));
    assert!(joiner.fallback().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_given_port_wins_and_is_not_traded() {
    let (port, given) = (free_port(), free_port());
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, Some(given)).await.unwrap();
    assert_eq!(joiner.local_addr().port(), u16::from(given));
    assert!(joiner.fallback().is_none());
    // Taken (by that joiner): an error, not another port.
    assert!(joiner_on(&share, Some(given)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_directory_gets_any_free_port() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    assert!(joiner.fallback().is_none());
}
