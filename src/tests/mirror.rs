//! The joiner serves a proxied site on the dev server's port when it can,
//! so the dev server's own scripts that dial that port reach it.

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
};

use crate::{
    core::port::Port,
    join::{Fallback, Unavailable},
    tests::harness::{fixture_dir, joiner_on, share_files, share_proxy, within},
};

/// A candidate free on both loopbacks now. It cannot be reserved across
/// Joiner::start, so callers must retry if someone takes it first.
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
    let (port, _share, joiner) = within("an uncontended mirrored port", async {
        loop {
            let port = free_port();
            let share = share_proxy(dev_server(port)).await;
            let joiner = joiner_on(&share, None).await.unwrap();
            match joiner.fallback() {
                Some(Fallback {
                    why: Unavailable::InUse,
                    ..
                }) => continue,
                Some(fallback) => panic!("{fallback}"),
                None => return (port, share, joiner),
            }
        }
    })
    .await;
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
    let _taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = _taken.local_addr().unwrap().port().try_into().unwrap();
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
    let Ok(_taken) = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)) else {
        return; // No IPv6 loopback here: nobody can take that side.
    };
    let port = _taken.local_addr().unwrap().port().try_into().unwrap();
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    assert_ne!(joiner.local_addr().port(), u16::from(port));
    assert!(joiner.fallback().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wildcard_listener_on_the_port_falls_back() {
    let _service = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = _service.local_addr().unwrap().port().try_into().unwrap();
    let share = share_proxy(dev_server(port)).await;
    let joiner = joiner_on(&share, None).await.unwrap();
    assert_ne!(joiner.local_addr().port(), u16::from(port));
    let Some(Fallback { wanted, why, .. }) = joiner.fallback() else {
        panic!("no fallback");
    };
    assert_eq!(*wanted, port);
    assert_eq!(*why, Unavailable::InUse);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_given_port_wins_and_is_not_traded() {
    // Keep the announced port occupied and distinct from the given one.
    let announced = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = announced.local_addr().unwrap().port().try_into().unwrap();
    let share = share_proxy(dev_server(port)).await;
    let (given, joiner) = within("an uncontended given port", async {
        loop {
            let given = free_port();
            match joiner_on(&share, Some(given)).await {
                Ok(joiner) => return (given, joiner),
                Err(error)
                    if error.iter_reports().any(|report| {
                        report
                            .downcast_current_context::<io::Error>()
                            .is_some_and(|error| error.kind() == io::ErrorKind::AddrInUse)
                    }) =>
                {
                    continue;
                }
                Err(error) => panic!("{error:?}"),
            }
        }
    })
    .await;
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
