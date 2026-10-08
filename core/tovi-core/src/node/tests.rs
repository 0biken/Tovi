//! Two real nodes in one process, talking QUIC over loopback.

use super::*;
use rand_core::{OsRng, RngCore};
use std::fs;
use tokio::sync::broadcast::error::RecvError;

struct TestNode {
    node: Node,
    events: broadcast::Receiver<Event>,
    dir: PathBuf,
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tovi-node-test-{:016x}", OsRng.next_u64()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path, name: &str) -> NodeConfig {
    let mut config = NodeConfig::new(dir.join("data"), name, dir.join("inbox"));
    // Loopback only: no firewall prompts, and no other machine can join in
    config.listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    config.approval_timeout = Duration::from_secs(2);
    config
}

async fn start(name: &str) -> TestNode {
    start_in(temp_dir(), name).await
}

async fn start_in(dir: PathBuf, name: &str) -> TestNode {
    let (node, events) = Node::start(config(&dir, name)).await.unwrap();
    TestNode { node, events, dir }
}

impl TestNode {
    /// Wait (up to 10 s) for the first event `pick` accepts
    async fn wait_for<T>(&mut self, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
        let wait = async {
            loop {
                match self.events.recv().await {
                    Ok(event) => {
                        if let Some(found) = pick(&event) {
                            return found;
                        }
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => panic!("event channel closed"),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .expect("expected event did not arrive")
    }

    async fn stop(self) {
        self.node.shutdown().await;
        drop(self.node);
        // Best effort: background tasks may still hold the database briefly
        let _ = fs::remove_dir_all(&self.dir);
    }

    fn file(&self, name: &str, len: usize) -> (PathBuf, Vec<u8>) {
        let mut data = vec![0u8; len];
        OsRng.fill_bytes(&mut data);
        let path = self.dir.join(name);
        fs::write(&path, &data).unwrap();
        (path, data)
    }
}

/// `phone` pairs with `desktop` using a fresh code; `allow` is the desktop
/// user's answer
async fn pair(desktop: &mut TestNode, phone: &TestNode, allow: bool) -> Result<TrustedDevice> {
    let code = desktop
        .node
        .new_pairing_code_with(vec![desktop.node.local_addr().unwrap()]);
    let uri = code.to_uri().unwrap();
    let phone_node = phone.node.clone();
    let pairing = tokio::spawn(async move { phone_node.pair(&uri).await });
    let request_id = desktop
        .wait_for(|e| match e {
            Event::PairingRequested { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .await;
    assert!(desktop.node.respond(request_id, allow));
    let paired = pairing.await.unwrap()?;
    // The desktop's side is done once it announces the pairing
    let phone_id = phone.node.device_id();
    desktop
        .wait_for(|e| match e {
            Event::Paired { device } if device.id == phone_id => Some(()),
            _ => None,
        })
        .await;
    Ok(paired)
}

fn finished_outcome(e: &Event, direction: Direction) -> Option<TransferOutcome> {
    match e {
        Event::TransferFinished {
            direction: d,
            outcome,
            ..
        } if *d == direction => Some(outcome.clone()),
        _ => None,
    }
}

#[tokio::test]
async fn pair_then_send_with_events_and_history() {
    let mut desktop = start("Desk").await;
    let mut phone = start("Phone").await;

    let paired = pair(&mut desktop, &phone, true).await.unwrap();
    assert_eq!(paired.id, desktop.node.device_id());
    assert_eq!(paired.name, "Desk");
    let desk_sees = &desktop.node.devices().unwrap()[0];
    assert_eq!(desk_sees.device.id, phone.node.device_id());
    assert_eq!(desk_sees.device.name, "Phone");
    assert!(
        !desk_sees.addresses.is_empty(),
        "desktop knows where to reach the phone"
    );

    let (source, data) = phone.file("holiday.jpg", 300 * 1024 + 9);
    let report = phone
        .node
        .send_file(&desktop.node.device_id(), &source)
        .await
        .unwrap();
    assert_eq!(report.size, data.len() as u64);
    assert_eq!(report.file_hash, blake3::hash(&data));

    // Receiver: started, then finished with the saved file
    let started = desktop
        .wait_for(|e| match e {
            Event::TransferStarted {
                direction: Direction::Received,
                file_name,
                device,
                ..
            } => Some((file_name.clone(), device.name.clone())),
            _ => None,
        })
        .await;
    assert_eq!(started, ("holiday.jpg".to_string(), "Phone".to_string()));
    let outcome = desktop
        .wait_for(|e| finished_outcome(e, Direction::Received))
        .await;
    let TransferOutcome::Completed {
        path, file_hash, ..
    } = outcome
    else {
        panic!("receive did not complete: {outcome:?}")
    };
    let path = path.unwrap();
    assert_eq!(path, desktop.dir.join("inbox").join("holiday.jpg"));
    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(file_hash, report.file_hash);

    // Sender: a final progress event at 100%, then finished
    let last = phone
        .wait_for(|e| match e {
            Event::TransferProgress {
                done,
                total,
                direction: Direction::Sent,
                ..
            } if done == total => Some(*total),
            _ => None,
        })
        .await;
    assert_eq!(last, data.len() as u64);
    assert!(matches!(
        phone
            .wait_for(|e| finished_outcome(e, Direction::Sent))
            .await,
        TransferOutcome::Completed { .. }
    ));

    // Both histories, both device lists
    let sent = &phone.node.history(10).unwrap()[0];
    assert_eq!(
        (sent.direction, sent.file_name.as_str()),
        (Direction::Sent, "holiday.jpg")
    );
    let received = &desktop.node.history(10).unwrap()[0];
    assert_eq!(received.direction, Direction::Received);
    assert_eq!(phone.node.devices().unwrap()[0].device.name, "Desk");
    assert_eq!(desktop.node.devices().unwrap()[0].device.name, "Phone");

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn declined_pairing_trusts_nobody() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;

    let err = pair(&mut desktop, &phone, false).await.unwrap_err();
    assert!(err.to_string().contains("declined"), "{err:#}");
    assert!(desktop.node.devices().unwrap().is_empty());
    assert!(phone.node.devices().unwrap().is_empty());

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn unanswered_pairing_request_expires() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    let code = desktop
        .node
        .new_pairing_code_with(vec![desktop.node.local_addr().unwrap()]);

    let started = Instant::now();
    let err = phone.node.pair(&code.to_uri().unwrap()).await.unwrap_err();
    assert!(err.to_string().contains("declined"), "{err:#}");
    assert!(started.elapsed() < Duration::from_secs(8));
    let requested = desktop
        .wait_for(|e| match e {
            Event::PairingRequested { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .await;
    let expired = desktop
        .wait_for(|e| match e {
            Event::RequestExpired { request_id } => Some(*request_id),
            _ => None,
        })
        .await;
    assert_eq!(expired, requested);
    assert!(!desktop.node.respond(requested, true), "too late to answer");
    assert!(desktop.node.devices().unwrap().is_empty());

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn with_auto_accept_off_each_file_is_asked_about() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    pair(&mut desktop, &phone, true).await.unwrap();
    assert!(desktop.node.auto_accept(), "auto-accept is on by default");
    desktop.node.set_auto_accept(false).unwrap();
    let (source, data) = phone.file("contract.pdf", 1000);
    let desk_id = desktop.node.device_id();

    for allow in [false, true] {
        let phone_node = phone.node.clone();
        let source = source.clone();
        let sending = tokio::spawn(async move { phone_node.send_file(&desk_id, &source).await });
        let (request_id, file_name) = desktop
            .wait_for(|e| match e {
                Event::IncomingOffer {
                    request_id,
                    file_name,
                    ..
                } => Some((*request_id, file_name.clone())),
                _ => None,
            })
            .await;
        assert_eq!(file_name, "contract.pdf");
        assert!(desktop.node.respond(request_id, allow));
        let result = sending.await.unwrap();
        if allow {
            result.unwrap();
        } else {
            let err = result.unwrap_err();
            assert!(err.to_string().contains("declined"), "{err:#}");
        }
    }
    let saved = desktop.dir.join("inbox").join("contract.pdf");
    assert_eq!(fs::read(saved).unwrap(), data);

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn forgotten_device_is_refused_without_retrying() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    pair(&mut desktop, &phone, true).await.unwrap();
    assert!(desktop.node.forget(&phone.node.device_id()).unwrap());

    let (source, _) = phone.file("note.txt", 10);
    let started = Instant::now();
    let err = phone
        .node
        .send_file(&desktop.node.device_id(), &source)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("refused"), "{err:#}");
    assert!(started.elapsed() < Duration::from_secs(5));

    desktop.stop().await;
    phone.stop().await;
}

/// The phone forgot the desktop, but the desktop still trusts the phone: the
/// phone opens with HELLO, not a file, and must be able to pair again
#[tokio::test]
async fn trusted_device_can_pair_again_after_forgetting() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    pair(&mut desktop, &phone, true).await.unwrap();
    assert!(phone.node.forget(&desktop.node.device_id()).unwrap());
    assert!(phone.node.devices().unwrap().is_empty());

    let paired = pair(&mut desktop, &phone, true).await.unwrap();
    assert_eq!(paired.id, desktop.node.device_id());
    let desk_sees = desktop.node.devices().unwrap();
    assert_eq!(desk_sees.len(), 1, "still one entry for the phone");
    assert_eq!(desk_sees[0].device.id, phone.node.device_id());

    let (source, data) = phone.file("again.bin", 64 * 1024 + 3);
    let report = phone
        .node
        .send_file(&desktop.node.device_id(), &source)
        .await
        .unwrap();
    assert_eq!(report.file_hash, blake3::hash(&data));
    let outcome = desktop
        .wait_for(|e| finished_outcome(e, Direction::Received))
        .await;
    let TransferOutcome::Completed { path, .. } = outcome else {
        panic!("receive did not complete: {outcome:?}")
    };
    assert_eq!(fs::read(path.unwrap()).unwrap(), data);

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn receive_folder_change_applies_and_persists() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    pair(&mut desktop, &phone, true).await.unwrap();

    let new_dir = desktop.dir.join("elsewhere");
    desktop.node.set_receive_dir(&new_dir).unwrap();
    assert_eq!(desktop.node.receive_dir(), new_dir);
    let (source, data) = phone.file("song.flac", 2000);
    phone
        .node
        .send_file(&desktop.node.device_id(), &source)
        .await
        .unwrap();
    let outcome = desktop
        .wait_for(|e| finished_outcome(e, Direction::Received))
        .await;
    let TransferOutcome::Completed { path, .. } = outcome else {
        panic!("receive did not complete: {outcome:?}")
    };
    assert_eq!(path.unwrap(), new_dir.join("song.flac"));
    assert_eq!(fs::read(new_dir.join("song.flac")).unwrap(), data);

    // The choice survives a restart
    let dir = desktop.dir.clone();
    desktop.node.shutdown().await;
    drop(desktop);
    let restarted = start_in(dir, "Desk").await;
    assert_eq!(restarted.node.receive_dir(), new_dir);

    restarted.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn pairing_survives_restart_and_works_both_ways() {
    let mut desktop = start("Desk").await;
    let phone = start("Phone").await;
    pair(&mut desktop, &phone, true).await.unwrap();

    // The desktop can send to the phone too: it learned the phone's address
    let (to_phone, data) = desktop.file("reply.txt", 500);
    desktop
        .node
        .send_file(&phone.node.device_id(), &to_phone)
        .await
        .unwrap();
    assert_eq!(
        fs::read(phone.dir.join("inbox").join("reply.txt")).unwrap(),
        data
    );

    // The phone restarts, on a new port; its pairing is still there
    let phone_dir = phone.dir.clone();
    phone.node.shutdown().await;
    drop(phone);
    let phone = start_in(phone_dir, "Phone").await;
    assert_eq!(
        phone.node.devices().unwrap()[0].device.id,
        desktop.node.device_id()
    );
    let (to_desk, _) = phone.file("again.txt", 100);
    phone
        .node
        .send_file(&desktop.node.device_id(), &to_desk)
        .await
        .unwrap();

    // ...and that connection taught the desktop the phone's new address
    let (to_phone, data) = desktop.file("reply2.txt", 600);
    desktop
        .node
        .send_file(&phone.node.device_id(), &to_phone)
        .await
        .unwrap();
    assert_eq!(
        fs::read(phone.dir.join("inbox").join("reply2.txt")).unwrap(),
        data
    );

    desktop.stop().await;
    phone.stop().await;
}

#[tokio::test]
async fn busy_port_falls_back_to_a_free_one() {
    let taken = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let busy = taken.local_addr().unwrap();
    let dir = temp_dir();
    let mut config = config(&dir, "Desk");
    config.listen = busy;

    let (node, _events) = Node::start(config).await.unwrap();
    let port = node.local_addr().unwrap().port();
    assert_ne!(port, busy.port());
    assert_ne!(port, 0);

    node.shutdown().await;
    drop(node);
    let _ = fs::remove_dir_all(dir);
}
