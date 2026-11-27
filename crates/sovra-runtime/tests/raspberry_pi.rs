//! Discovery uses the real runtime without holding up other UI commands.

use sovra_core::{
    Command, Event,
    mcp::{McpCommand, McpEvent},
    raspberry_pi::DiscoveryEvent,
};
use std::time::Duration;

#[tokio::test]
async fn raspberry_pi_discovery_finishes_and_keeps_the_client_responsive() {
    let client = sovra_runtime::start("pi-discovery-test").expect("runtime");
    let events = client.events();
    client
        .send(Command::Mcp(McpCommand::DiscoverRaspberryPis))
        .expect("search");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if matches!(
                events.recv().await.expect("event"),
                Event::Mcp(McpEvent::RaspberryPiDiscovery(DiscoveryEvent::Started))
            ) {
                break;
            }
        }
    })
    .await
    .expect("search starts");
    client.send(Command::Ping).expect("ping during search");
    let mut finished = None;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await.expect("event") {
                Event::Pong => break,
                Event::Mcp(McpEvent::RaspberryPiDiscovery(DiscoveryEvent::Finished(result))) => {
                    finished = Some(result);
                }
                _ => {}
            }
        }
    })
    .await
    .expect("search must not block the client");
    let result = tokio::time::timeout(Duration::from_secs(12), async {
        if let Some(result) = finished {
            return result;
        }
        loop {
            if let Event::Mcp(McpEvent::RaspberryPiDiscovery(DiscoveryEvent::Finished(result))) =
                events.recv().await.expect("event")
            {
                return result;
            }
        }
    })
    .await
    .expect("bounded discovery result");
    // Network-less CI may report an actionable error. It must never hang or invent a Pi.
    match result {
        Ok(candidates) => assert!(candidates.len() <= 32),
        Err(problem) => assert!(problem.contains("network")),
    }
    client.send(Command::Shutdown).expect("stop");
}

#[tokio::test]
async fn pairing_validation_reports_a_failure_and_keeps_secrets_out_of_events() {
    use sovra_core::{
        models::ModelSecret,
        raspberry_pi::{PairingEvent, PiSetup},
    };
    let client = sovra_runtime::start("pi-pairing-test").expect("runtime");
    let events = client.events();
    let command = McpCommand::PairRaspberryPi {
        setup: PiSetup {
            board_id: "pico".into(),
            host: String::new(),
            user: String::new(),
        },
        password: ModelSecret("private-test-password".into()),
    };
    assert!(!format!("{command:?}").contains("private-test-password"));
    client.send(Command::Mcp(command)).expect("pair");
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut started = None;
        loop {
            let event = events.recv().await.expect("event");
            assert!(!format!("{event:?}").contains("private-test-password"));
            match event {
                Event::Mcp(McpEvent::RaspberryPiPairing(PairingEvent::Started {
                    request_id,
                    ..
                })) => started = Some(request_id),
                Event::Mcp(McpEvent::RaspberryPiPairing(PairingEvent::Finished {
                    request_id,
                    result,
                })) => {
                    assert_eq!(started, Some(request_id));
                    assert!(
                        result
                            .expect_err("Pico does not use password")
                            .contains("USB")
                    );
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("bounded pairing failure");
    client.send(Command::Ping).expect("client still responsive");
    tokio::time::timeout(Duration::from_secs(2), async {
        while !matches!(events.recv().await.expect("event"), Event::Pong) {}
    })
    .await
    .expect("pong");
    client.send(Command::Shutdown).expect("stop");
}
