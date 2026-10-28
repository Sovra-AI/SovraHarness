//! A busy decoder receives control commands without polling the scan loop.

use super::*;

#[test]
fn decoder_cancellation_reaches_cancel_pause_and_shutdown() {
    for command in [
        Command::FileIndex(IndexCommand::Cancel {
            root_id: "root".into(),
        }),
        Command::FileIndex(IndexCommand::SetAutoRefresh {
            root_id: "root".into(),
            enabled: false,
        }),
        Command::Shutdown,
    ] {
        let shutdown = matches!(command, Command::Shutdown);
        let (_release, receiver) = mpsc::channel();
        let store = Arc::new(Store {
            decoder: true,
            release: Mutex::new(receiver),
            proposals: Mutex::new(vec![]),
            root: Mutex::new(IndexedRoot {
                id: "root".into(),
                path: "/chosen".into(),
                status: IndexStatus::Pending,
                progress: IndexProgress::default(),
                issues: vec![],
                completed_at: None,
                auto_refresh: false,
                scan_options: sovra_core::file_index::ScanOptions::default(),
            }),
        });
        let client = sovra_runtime::start_with_file_index(
            "decoder-test",
            AgentConfig::default(),
            None,
            None,
            None,
            None,
            Some(store.clone()),
        )
        .unwrap();
        let events = client.events();
        client
            .send(Command::FileIndex(IndexCommand::Refresh {
                root_id: "root".into(),
            }))
            .unwrap();
        loop {
            match events.recv_blocking().unwrap() {
                Event::FileIndex(IndexEvent::Progress(root))
                    if root.status == IndexStatus::Scanning =>
                {
                    break;
                }
                Event::FileIndex(IndexEvent::Error(message)) => panic!("{message}"),
                _ => {}
            }
        }
        client.send(Command::Ping).unwrap();
        loop {
            if events.recv_blocking().unwrap() == Event::Pong {
                break;
            }
        }
        client.send(command).unwrap();
        loop {
            match events.recv_blocking().unwrap() {
                Event::Stopped if shutdown => break,
                Event::FileIndex(IndexEvent::Progress(root))
                    if !shutdown && root.status == IndexStatus::Cancelled =>
                {
                    break;
                }
                Event::FileIndex(IndexEvent::Error(message)) => panic!("{message}"),
                _ => {}
            }
        }
        assert_eq!(
            store.root.lock().unwrap().status,
            if shutdown {
                IndexStatus::Interrupted
            } else {
                IndexStatus::Cancelled
            }
        );
        if !shutdown {
            client.send(Command::Shutdown).unwrap();
        }
    }
}
