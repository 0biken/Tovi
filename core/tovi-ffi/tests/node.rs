//! The bindings end to end over loopback QUIC: a `ToviNode` (the phone, as
//! Kotlin would drive it) pairs with a plain core `Node` (the desktop) and
//! files go both ways.

use rand_core::{OsRng, RngCore};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;
use tovi_core::node::{self, Event, Node};
use tovi_ffi::{
    Direction, EventListener, NodeConfig, NodeEvent, ToviError, ToviNode, TransferOutcome,
    TransferStatus,
};

const WAIT: Duration = Duration::from_secs(10);

/// What the Kotlin side would do: hand each event to a channel
struct ChannelListener(Mutex<Sender<NodeEvent>>);

impl EventListener for ChannelListener {
    fn on_event(&self, event: NodeEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

struct Phone {
    node: Arc<ToviNode>,
    events: Receiver<NodeEvent>,
}

impl Phone {
    fn start(dir: &Path) -> Self {
        let (tx, events) = mpsc::channel();
        let node = ToviNode::start(
            NodeConfig {
                data_dir: dir.join("phone").to_string_lossy().into_owned(),
                device_name: "Test Phone".into(),
                default_receive_dir: dir.join("phone-inbox").to_string_lossy().into_owned(),
                listen: Some("127.0.0.1:0".into()),
            },
            Arc::new(ChannelListener(Mutex::new(tx))),
        )
        .unwrap();
        Self { node, events }
    }

    /// Wait (up to 10 s) for the first event `pick` accepts
    fn wait_for<T>(&self, mut pick: impl FnMut(&NodeEvent) -> Option<T>) -> T {
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let event = self
                .events
                .recv_timeout(left)
                .expect("expected event did not arrive");
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    }
}

/// Wait on the desktop's events for the first one `pick` accepts
async fn desktop_wait<T>(
    events: &mut broadcast::Receiver<Event>,
    mut pick: impl FnMut(&Event) -> Option<T>,
) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(found) = pick(&event) {
                        return found;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => panic!("event channel closed"),
            }
        }
    })
    .await
    .expect("expected desktop event did not arrive")
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tovi-ffi-test-{:016x}", OsRng.next_u64()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn random_file(path: &Path, len: usize) -> Vec<u8> {
    let mut data = vec![0u8; len];
    OsRng.fill_bytes(&mut data);
    fs::write(path, &data).unwrap();
    data
}

fn start_desktop(rt: &Runtime, dir: &Path) -> (Node, broadcast::Receiver<Event>) {
    let mut config = node::NodeConfig::new(
        dir.join("desktop"),
        "Test Desktop",
        dir.join("desktop-inbox"),
    );
    config.listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    rt.block_on(Node::start(config)).unwrap()
}

/// The phone scans the desktop's code; the desktop user allows it
fn pair(
    rt: &Runtime,
    phone: &Phone,
    desktop: &Node,
    desktop_events: &mut broadcast::Receiver<Event>,
) {
    let uri = desktop
        .new_pairing_code_with(vec![desktop.local_addr().unwrap()])
        .to_uri()
        .unwrap();
    let approver = {
        let desktop = desktop.clone();
        let mut events = desktop_events.resubscribe();
        rt.spawn(async move {
            let request_id = desktop_wait(&mut events, |e| match e {
                Event::PairingRequested { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .await;
            assert!(desktop.respond(request_id, true));
        })
    };
    let device = rt.block_on(phone.node.pair(uri)).unwrap();
    rt.block_on(approver).unwrap();

    assert_eq!(device.id, desktop.device_id().to_string());
    assert_eq!(device.name, "Test Desktop");
    assert_eq!(
        device.address,
        Some(desktop.local_addr().unwrap().to_string())
    );
    let paired = phone.wait_for(|e| match e {
        NodeEvent::Paired { device_id, .. } => Some(device_id.clone()),
        _ => None,
    });
    assert_eq!(paired, device.id);
    assert_eq!(phone.node.devices().unwrap().len(), 1);
}

#[test]
fn phone_pairs_and_transfers_both_ways() {
    let dir = temp_dir();
    let rt = Runtime::new().unwrap();
    let (desktop, mut desktop_events) = start_desktop(&rt, &dir);
    let phone = Phone::start(&dir);
    let desktop_id = desktop.device_id().to_string();
    let phone_id = phone.node.local_id().device_id;

    pair(&rt, &phone, &desktop, &mut desktop_events);

    // Phone → desktop (the desktop auto-accepts by default)
    let outgoing = dir.join("photo.jpg");
    let sent = random_file(&outgoing, 300_000);
    let result = rt
        .block_on(
            phone
                .node
                .send_file(desktop_id.clone(), outgoing.to_string_lossy().into_owned()),
        )
        .unwrap();
    assert_eq!(result.size, sent.len() as u64);
    assert_eq!(fs::read(dir.join("desktop-inbox/photo.jpg")).unwrap(), sent);
    let finished = phone.wait_for(|e| match e {
        NodeEvent::TransferFinished {
            transfer_id,
            direction: Direction::Sent,
            outcome: TransferOutcome::Completed { .. },
            ..
        } => Some(transfer_id.clone()),
        _ => None,
    });
    assert_eq!(finished, result.transfer_id);

    // Desktop → phone, with auto-accept off: the phone is asked first
    phone.node.set_auto_accept(false).unwrap();
    assert!(!phone.node.auto_accept());
    let incoming = dir.join("notes.txt");
    let received = random_file(&incoming, 50_000);
    let send = {
        let desktop = desktop.clone();
        let to = phone_id.parse().unwrap();
        rt.spawn(async move { desktop.send_file(&to, &incoming).await })
    };
    let request_id = phone.wait_for(|e| match e {
        NodeEvent::IncomingOffer {
            request_id,
            device_id,
            file_name,
            file_size,
            ..
        } => {
            assert_eq!(*device_id, desktop_id);
            assert_eq!(file_name, "notes.txt");
            assert_eq!(*file_size, received.len() as u64);
            Some(*request_id)
        }
        _ => None,
    });
    assert!(phone.node.respond(request_id, true));
    rt.block_on(send).unwrap().unwrap();
    let saved = phone.wait_for(|e| match e {
        NodeEvent::TransferFinished {
            direction: Direction::Received,
            outcome: TransferOutcome::Completed { path, .. },
            ..
        } => Some(path.clone().expect("received files report their path")),
        _ => None,
    });
    assert_eq!(fs::read(&saved).unwrap(), received);
    assert!(Path::new(&saved).starts_with(phone.node.receive_dir()));

    // History, newest first, with the desktop's name
    let history = phone.node.history(10).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].direction, Direction::Received);
    assert_eq!(history[1].direction, Direction::Sent);
    assert!(history
        .iter()
        .all(|t| t.status == TransferStatus::Completed && t.device_name == "Test Desktop"));

    // Forgetting the desktop
    assert!(phone.node.forget(desktop_id.clone()).unwrap());
    assert!(phone.node.devices().unwrap().is_empty());
    phone.wait_for(|e| matches!(e, NodeEvent::DevicesChanged).then_some(()));

    rt.block_on(phone.node.shutdown());
    rt.block_on(desktop.shutdown());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bad_input_is_reported_as_invalid_argument() {
    let dir = temp_dir();
    let rt = Runtime::new().unwrap();
    let phone = Phone::start(&dir);

    assert!(matches!(
        phone.node.forget("not-an-id".into()),
        Err(ToviError::InvalidArgument { .. })
    ));
    assert!(matches!(
        rt.block_on(phone.node.send_file("abc".into(), "/tmp/x".into())),
        Err(ToviError::InvalidArgument { .. })
    ));
    assert!(matches!(
        phone.node.set_receive_dir("relative/folder".into()),
        Err(ToviError::InvalidArgument { .. })
    ));
    // A malformed code is a failed pairing, with a message for the user
    match rt.block_on(phone.node.pair("tovi://pair/garbage".into())) {
        Err(ToviError::Failed { message }) => assert!(!message.is_empty()),
        other => panic!("expected Failed, got {other:?}"),
    }

    rt.block_on(phone.node.shutdown());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn start_rejects_a_relative_receive_folder() {
    let (tx, _events) = mpsc::channel();
    let result = ToviNode::start(
        NodeConfig {
            data_dir: temp_dir().to_string_lossy().into_owned(),
            device_name: "Phone".into(),
            default_receive_dir: "Downloads/TOVI".into(),
            listen: Some("127.0.0.1:0".into()),
        },
        Arc::new(ChannelListener(Mutex::new(tx))),
    );
    assert!(matches!(result, Err(ToviError::InvalidArgument { .. })));
}
