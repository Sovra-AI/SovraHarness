//! Bounded local discovery. Advertisements never authenticate a Raspberry Pi.

use std::{collections::BTreeSet, time::Duration};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use sovra_core::raspberry_pi::NetworkCandidate;

const SEARCH_TIME: Duration = Duration::from_secs(8);
const NETWORK_PROBLEM: &str = "Couldn't search this network. Allow local network access for Sovra, or enter your Pi's computer name below.";

struct DiscoveryDaemon(ServiceDaemon);

impl Drop for DiscoveryDaemon {
    fn drop(&mut self) {
        // Also runs when the owning runtime task is cancelled.
        let _ = self.0.shutdown();
    }
}

pub(super) async fn discover() -> Result<Vec<NetworkCandidate>, String> {
    let daemon = tokio::task::spawn_blocking(|| ServiceDaemon::new().map(DiscoveryDaemon))
        .await
        .map_err(|_| NETWORK_PROBLEM)?
        .map_err(|_| NETWORK_PROBLEM)?;
    let ssh = daemon
        .0
        .browse("_ssh._tcp.local.")
        .map_err(|_| NETWORK_PROBLEM)?;
    // Avahi may publish a workstation without publishing SSH separately.
    let workstations = daemon
        .0
        .browse("_workstation._tcp.local.")
        .map_err(|_| NETWORK_PROBLEM)?;
    let mut found = BTreeSet::new();
    let deadline = tokio::time::sleep(SEARCH_TIME);
    tokio::pin!(deadline);
    let default_host = tokio::net::TcpStream::connect(("raspberrypi.local", 22));
    tokio::pin!(default_host);
    let mut checked_default = false;
    loop {
        let event = tokio::select! {
            () = &mut deadline => break,
            result = &mut default_host, if !checked_default => {
                checked_default = true;
                if result.is_ok() {
                    found.insert(NetworkCandidate::from_hostname("raspberrypi.local").expect("fixed local hostname"));
                }
                if found.len() >= 32 { break; }
                continue;
            }
            event = ssh.recv_async() => event,
            event = workstations.recv_async() => event,
        }.map_err(|_| NETWORK_PROBLEM)?;
        if let ServiceEvent::ServiceResolved(service) = event
            && let Some(candidate) = NetworkCandidate::from_hostname(service.get_hostname())
        {
            found.insert(candidate);
        }
        if found.len() >= 32 {
            break;
        }
    }
    if let Ok(stopped) = daemon.0.shutdown() {
        let _ = tokio::time::timeout(Duration::from_secs(1), stopped.recv_async()).await;
    }
    Ok(found.into_iter().collect())
}
