//! Cancellation and UI responsiveness cross the real Client boundary.

impl FileKnowledge for Store {
    fn propose(&self, _: &GraphDraft, _: &GraphOrigin) -> Result<KnowledgeProposal, IndexError> {
        Err(IndexError("Not used by this command fixture".into()))
    }
    fn pending(&self) -> Result<Vec<KnowledgeProposal>, IndexError> {
        Ok(self
            .proposals
            .lock()
            .expect("proposal fixture lock")
            .clone())
    }
    fn decide(&self, id: u64, approve: bool) -> Result<KnowledgeProposal, IndexError> {
        let mut proposals = self.proposals.lock().expect("proposal fixture lock");
        let index = proposals
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| IndexError("Already reviewed".into()))?;
        let mut proposal = proposals.remove(index);
        proposal.state = if approve {
            ProposalState::Accepted
        } else {
            ProposalState::Rejected
        };
        Ok(proposal)
    }
    fn query(&self, _: &GraphQuery) -> Result<GraphPage, IndexError> {
        Ok(GraphPage {
            entities: vec![],
            relations: vec![],
            sources: vec![],
            truncated: false,
        })
    }
    fn evidence(&self, _: u64) -> Result<GraphEvidence, IndexError> {
        Err(IndexError("No evidence".into()))
    }
}

#[test]
fn knowledge_review_flows_through_client_and_duplicate_review_reports_an_error() {
    let (_release, receiver) = mpsc::channel();
    let p = KnowledgeProposal {
        id: 1,
        state: ProposalState::Pending,
        created_at: 1,
        reviewed_at: None,
        origin: GraphOrigin {
            session_id: "s".into(),
            invocation_id: "i".into(),
            agent: "helper".into(),
        },
        draft: GraphDraft {
            title: "Fixture".into(),
            evidence: vec![],
            entities: vec![],
            relations: vec![],
        },
        sources: vec![],
        existing_entities: vec![],
        replaced_relations: vec![],
        required_roots: vec!["root".into()],
    };
    let mut p2 = p.clone();
    p2.id = 2;
    let store = Arc::new(Store {
        decoder: false,
        release: Mutex::new(receiver),
        proposals: Mutex::new(vec![p, p2]),
        root: Mutex::new(IndexedRoot {
            id: "root".into(),
            path: "/chosen".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(1),
            auto_refresh: false,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        }),
    });
    let client = sovra_runtime::start_with_file_index(
        "review-test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        Some(store.clone()),
    )
    .unwrap();
    let events = client.events();
    let mut roots_seen = false;
    loop {
        match events.recv_blocking().unwrap() {
            Event::FileIndex(IndexEvent::Roots(_)) => roots_seen = true,
            Event::FileIndex(IndexEvent::KnowledgePending(proposals)) => {
                assert!(roots_seen);
                assert_eq!(proposals.len(), 2);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(store.pending().unwrap().len(), 2);
    for (id, approve) in [(1, true), (2, false)] {
        client
            .send(Command::FileIndex(IndexCommand::ReviewKnowledge {
                proposal_id: id,
                approve,
            }))
            .unwrap();
        loop {
            if let Event::FileIndex(IndexEvent::KnowledgeReviewed {
                proposal_id,
                accepted,
            }) = events.recv_blocking().unwrap()
            {
                assert_eq!((proposal_id, accepted), (id, approve));
                break;
            }
        }
    }
    assert_eq!(store.pending().unwrap(), Vec::<KnowledgeProposal>::new());
    client
        .send(Command::FileIndex(IndexCommand::ReviewKnowledge {
            proposal_id: 1,
            approve: true,
        }))
        .unwrap();
    loop {
        if let Event::FileIndex(IndexEvent::Error(message)) = events.recv_blocking().unwrap() {
            assert!(message.contains("Already reviewed"));
            break;
        }
    }
    client.send(Command::Ping).unwrap();
    loop {
        if events.recv_blocking().unwrap() == Event::Pong {
            break;
        }
    }
    client.send(Command::Shutdown).unwrap();
}

#[path = "file_index/monitoring.rs"]
mod monitoring;

#[path = "file_index/batch.rs"]
mod batch;

use sovra_core::file_index::graph::*;
use sovra_core::{
    AgentConfig, Command, Event,
    file_index::{
        CatalogPage, CatalogQuery, FileIndex, IndexCommand, IndexError, IndexEvent, IndexPassage,
        IndexProgress, IndexQuery, IndexStatus, IndexedRoot, PassageRef, ScanObserver,
    },
};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

#[derive(Debug)]
struct Store {
    decoder: bool,
    proposals: Mutex<Vec<KnowledgeProposal>>,
    release: Mutex<mpsc::Receiver<()>>,
    root: Mutex<IndexedRoot>,
}

impl FileIndex for Store {
    fn report(
        &self,
        query: &sovra_core::file_index::NoticeQuery,
    ) -> Result<sovra_core::file_index::NoticePage, IndexError> {
        query.validate()?;
        Ok(sovra_core::file_index::NoticePage {
            root_id: query.root_id.clone(),
            scan_id: Some(1),
            status: IndexStatus::Scanning,
            total: 0,
            has_newer: false,
            counts: vec![],
            notices: vec![],
            next_cursor: None,
        })
    }
    fn knowledge(&self) -> Option<&dyn FileKnowledge> {
        Some(self)
    }
    fn history(
        &self,
        _: &sovra_core::file_index::FileHistoryQuery,
    ) -> Result<sovra_core::file_index::FileHistory, IndexError> {
        Err(IndexError("No history in this scan fixture.".into()))
    }
    fn read_history(
        &self,
        _: &sovra_core::file_index::HistoryPassageRef,
    ) -> Result<sovra_core::file_index::HistoricalPassage, IndexError> {
        Err(IndexError("No history in this scan fixture.".into()))
    }
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        Ok(vec![self.root.lock().expect("index root lock").clone()])
    }
    fn recover(&self) -> Result<(), IndexError> {
        Ok(())
    }
    fn set_auto_refresh(&self, _: &str, enabled: bool) -> Result<(), IndexError> {
        self.root.lock().expect("index root lock").auto_refresh = enabled;
        Ok(())
    }
    fn watch(
        &self,
        _: &IndexedRoot,
        _: Arc<dyn sovra_core::file_index::IndexChanges>,
    ) -> Result<Box<dyn sovra_core::file_index::IndexWatch>, IndexError> {
        Err(IndexError(
            "Test scope uses periodic reconciliation.".into(),
        ))
    }
    fn add_root(&self, _: &Path) -> Result<IndexedRoot, IndexError> {
        Ok(self.root.lock().expect("index root lock").clone())
    }
    fn scan(&self, _: &str, observer: &dyn ScanObserver) -> Result<IndexedRoot, IndexError> {
        let mut root = self.root.lock().expect("index root lock").clone();
        root.status = IndexStatus::Scanning;
        if self.decoder {
            // Real file decoders run on scoped reader threads, outside Tokio's
            // blocking pool. Their cancellation callback must work there too.
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let (obsolete, obsolete_events) = mpsc::channel();
                        let watch = observer.watch_cancellation(Arc::new(move || {
                            let _ = obsolete.send(());
                        }));
                        assert!(watch.is_some());
                        drop(watch);
                        let (cancel, cancelled) = mpsc::channel();
                        let _watch = observer.watch_cancellation(Arc::new(move || {
                            let _ = cancel.send(());
                        }));
                        observer.progress(&root);
                        cancelled
                            .recv_timeout(std::time::Duration::from_secs(3))
                            .expect(
                                "decoder receives cancellation without returning to the scan loop",
                            );
                        assert!(obsolete_events.try_recv().is_err());
                    })
                    .join()
                    .expect("decoder thread must not panic");
            });
        } else {
            observer.progress(&root);
            self.release
                .lock()
                .expect("scan release lock")
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("scan release");
        }
        assert!(observer.cancelled());
        root.status = match observer.cancellation().expect("scan is cancelled") {
            sovra_core::file_index::ScanCancellation::Requested => IndexStatus::Cancelled,
            sovra_core::file_index::ScanCancellation::Interrupted => IndexStatus::Interrupted,
        };
        *self.root.lock().expect("index root lock") = root.clone();
        observer.progress(&root);
        Ok(root)
    }
    fn remove(&self, _: &str) -> Result<(), IndexError> {
        Ok(())
    }
    fn search(&self, _: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        Ok(vec![])
    }
    fn read(&self, _: &PassageRef) -> Result<IndexPassage, IndexError> {
        Err(IndexError("Not found".into()))
    }
    fn catalogue(&self, query: &CatalogQuery) -> Result<CatalogPage, IndexError> {
        query.validate()?;
        Ok(CatalogPage {
            revision: 0,
            total_files: 0,
            total_known_bytes: 0,
            unknown_sizes: 0,
            coverage: vec![],
            files: Vec::new(),
            groups: Vec::new(),
            next_cursor: None,
            has_more: false,
        })
    }
}

#[test]
fn scans_leave_ping_and_search_responsive_and_cancel_reaches_the_blocking_worker() {
    let (release, receiver) = mpsc::channel();
    let store = Arc::new(Store {
        decoder: false,
        proposals: Mutex::new(vec![]),
        release: Mutex::new(receiver),
        root: Mutex::new(IndexedRoot {
            id: "root".into(),
            path: "/selected".into(),
            status: IndexStatus::Pending,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: false,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        }),
    });
    let client = sovra_runtime::start_with_file_index(
        "test",
        AgentConfig::default(),
        None,
        None,
        None,
        None,
        Some(store),
    )
    .unwrap();
    let events = client.events();
    client
        .send(Command::FileIndex(IndexCommand::Refresh {
            root_id: "root".into(),
        }))
        .unwrap();
    loop {
        if matches!(events.recv_blocking().unwrap(), Event::FileIndex(IndexEvent::Progress(root)) if root.status == IndexStatus::Scanning)
        {
            break;
        }
    }
    client.send(Command::Ping).unwrap();
    loop {
        if events.recv_blocking().unwrap() == Event::Pong {
            break;
        }
    }
    client
        .send(Command::FileIndex(IndexCommand::Search {
            request_id: 7,
            query: IndexQuery {
                text: "needle".into(),
                roots: vec![],
                path_prefix: None,
                limit: 4,
            },
        }))
        .unwrap();
    loop {
        if matches!(
            events.recv_blocking().unwrap(),
            Event::FileIndex(IndexEvent::Results { request_id: 7, .. })
        ) {
            break;
        }
    }
    // Inventory and saved-file reads remain responsive while scanning is held.
    for (request_id, limit) in [(81, 20), (82, 0)] {
        client
            .send(Command::FileIndex(IndexCommand::Browse {
                request_id,
                query: CatalogQuery {
                    limit,
                    ..CatalogQuery::default()
                },
            }))
            .unwrap();
        loop {
            if let Event::FileIndex(IndexEvent::Browsed {
                request_id: received,
                result,
            }) = events.recv_blocking().unwrap()
            {
                assert_eq!(received, request_id);
                assert_eq!(result.is_ok(), limit > 0);
                break;
            }
        }
    }
    client
        .send(Command::FileIndex(IndexCommand::Preview {
            request_id: 83,
            reference: PassageRef {
                root_id: "root".into(),
                file_id: "missing".into(),
                version_id: "missing".into(),
                ordinal: 0,
            },
        }))
        .unwrap();
    loop {
        if let Event::FileIndex(IndexEvent::Previewed { request_id, result }) =
            events.recv_blocking().unwrap()
        {
            assert_eq!(request_id, 83);
            assert_eq!(result.unwrap_err().0, "Not found");
            break;
        }
    }
    for (id, limit) in [(85, 40), (86, 0)] {
        client
            .send(Command::FileIndex(IndexCommand::Report {
                request_id: id,
                query: sovra_core::file_index::NoticeQuery {
                    root_id: "root".into(),
                    kind: None,
                    contains: None,
                    match_case: false,
                    cursor: None,
                    limit,
                },
            }))
            .unwrap();
        loop {
            if let Event::FileIndex(IndexEvent::Reported { request_id, result }) =
                events.recv_blocking().unwrap()
            {
                assert_eq!(request_id, id);
                assert_eq!(result.is_ok(), limit > 0);
                break;
            }
        }
    }
    client
        .send(Command::FileIndex(IndexCommand::Cancel {
            root_id: "root".into(),
        }))
        .unwrap();
    client.send(Command::Ping).unwrap();
    loop {
        if events.recv_blocking().unwrap() == Event::Pong {
            break;
        }
    }
    release.send(()).unwrap();
    loop {
        if matches!(events.recv_blocking().unwrap(), Event::FileIndex(IndexEvent::Progress(root)) if root.status == IndexStatus::Cancelled)
        {
            break;
        }
    }
    client.send(Command::Shutdown).unwrap();
}

#[path = "file_index/cancellation.rs"]
mod cancellation;
