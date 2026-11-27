//! Nearby computer names are suggestions, never proof of a Pi or trusted identity.

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NetworkCandidate {
    host: String,
}

impl NetworkCandidate {
    pub fn from_hostname(host: &str) -> Option<Self> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let label = host.strip_suffix(".local")?;
        // A local name cannot introduce a shell option, remote domain or UI control text.
        if label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes().last()?.is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        {
            return None;
        }
        Some(Self { host })
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Started,
    Finished(Result<Vec<NetworkCandidate>, String>),
}

#[derive(Debug, Clone, Default)]
pub struct DiscoveryState {
    pub searching: bool,
    pub searched: bool,
    pub candidates: Vec<NetworkCandidate>,
    pub problem: Option<String>,
}

impl DiscoveryState {
    pub fn apply(&mut self, event: &DiscoveryEvent) {
        match event {
            DiscoveryEvent::Started => {
                self.searching = true;
                self.searched = false;
                self.candidates.clear();
                self.problem = None;
            }
            DiscoveryEvent::Finished(result) => {
                self.searching = false;
                self.searched = true;
                match result {
                    Ok(candidates) => {
                        self.candidates.clone_from(candidates);
                        self.candidates.sort();
                        self.candidates.dedup();
                        self.candidates.truncate(32);
                        self.problem = None;
                    }
                    Err(problem) => {
                        self.candidates.clear();
                        self.problem = Some(problem.clone());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_names_are_only_bounded_local_hostnames() {
        assert_eq!(
            NetworkCandidate::from_hostname("GardenPi.LOCAL.")
                .unwrap()
                .host(),
            "gardenpi.local"
        );
        for name in [
            ".local",
            "-o.local",
            "pi-.local",
            "pi.local.evil.com",
            "pi\n.local",
            "a.b.local",
            "$(whoami).local",
            "pi@home.local",
        ] {
            assert!(NetworkCandidate::from_hostname(name).is_none(), "{name}");
        }
        assert!(NetworkCandidate::from_hostname(&format!("{}.local", "p".repeat(64))).is_none());
    }

    #[test]
    fn a_new_search_clears_old_results_and_bounds_duplicates() {
        let mut state = DiscoveryState::default();
        let candidates = (0..40)
            .map(|i| NetworkCandidate::from_hostname(&format!("pi-{i}.local")).unwrap())
            .collect();
        state.apply(&DiscoveryEvent::Finished(Ok(candidates)));
        assert_eq!(state.candidates.len(), 32);
        state.apply(&DiscoveryEvent::Started);
        assert!(state.searching && state.candidates.is_empty() && !state.searched);
        let pi = NetworkCandidate::from_hostname("pi.local").unwrap();
        state.apply(&DiscoveryEvent::Finished(Ok(vec![pi.clone(), pi])));
        assert_eq!(state.candidates.len(), 1);
        assert!(!state.searching && state.searched);
        state.apply(&DiscoveryEvent::Finished(Err("Network unavailable".into())));
        assert_eq!(state.candidates, []);
        assert_eq!(state.problem.as_deref(), Some("Network unavailable"));
    }
}
