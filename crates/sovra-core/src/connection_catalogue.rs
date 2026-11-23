//! The Connections page: one closed list of the links this app ships.
//!
//! A new kind is one [`CatalogueKind`] variant plus one row in [`KINDS`]. It does not
//! add a sidebar item or a Settings section. Each card's status
//! is independent: folding one kind cannot change another.
//!
//! The first screen is everyday words only. A token, secret, client id, or base URL
//! appears after a person opens a card, in the editor that already existed.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::connections::{TelegramStatus, WhatsAppStatus};
use crate::github::GitHubState;
use crate::google::GoogleState;
use crate::mcp::{McpPhase, McpService};
use crate::microsoft::MicrosoftState;
use crate::models::{CatalogState, ModelConnection, ProviderKind};
use crate::storage::{PlaceStatus, StorageBook};

/// How many installed links a refresh may probe at once.
///
/// The same bound ADK uses for a delegation batch: enough to overlap, small enough
/// that one stalled service cannot pile up unbounded work.
pub const PROBE_BOUND: usize = 4;

/// Words the first screen must not ask for. The opened card may still collect them.
const FIRST_SCREEN_FORBIDDEN: [&str; 4] = ["token", "secret", "client id", "base url"];

/// A link the app can show. Order is the order of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CatalogueKind {
    OnlineAi,
    LocalAi,
    CompatibleService,
    WhatsApp,
    Telegram,
    Google,
    GitHub,
    Microsoft,
    Storage,
    WebAddress,
}

impl CatalogueKind {
    pub const ALL: [Self; 10] = [
        Self::OnlineAi,
        Self::LocalAi,
        Self::CompatibleService,
        Self::WhatsApp,
        Self::Telegram,
        Self::Google,
        Self::GitHub,
        Self::Microsoft,
        Self::Storage,
        Self::WebAddress,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::OnlineAi => "online-ai",
            Self::LocalAi => "local-ai",
            Self::CompatibleService => "compatible",
            Self::WhatsApp => "whatsapp",
            Self::Telegram => "telegram",
            Self::Google => "google",
            Self::GitHub => "github",
            Self::Microsoft => "microsoft",
            Self::Storage => "storage",
            Self::WebAddress => "web-address",
        }
    }

    /// The section of the page this kind sits in.
    pub const fn group(self) -> CatalogueGroup {
        match self {
            Self::OnlineAi | Self::LocalAi | Self::CompatibleService => CatalogueGroup::Ai,
            Self::WhatsApp | Self::Telegram => CatalogueGroup::Chat,
            Self::Google | Self::GitHub | Self::Microsoft => CatalogueGroup::Accounts,
            Self::Storage => CatalogueGroup::Storage,
            Self::WebAddress => CatalogueGroup::Tools,
        }
    }
}

/// A section of the Connections page. Order is the order of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CatalogueGroup {
    Ai,
    Chat,
    Accounts,
    Storage,
    Tools,
}

impl CatalogueGroup {
    pub const ALL: [Self; 5] = [
        Self::Ai,
        Self::Chat,
        Self::Accounts,
        Self::Storage,
        Self::Tools,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Chat => "chat",
            Self::Accounts => "accounts",
            Self::Storage => "storage",
            Self::Tools => "tools",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Ai => "AI that answers you",
            Self::Chat => "Chat apps",
            Self::Accounts => "Your accounts",
            Self::Storage => "Files and storage",
            Self::Tools => "Extra tools",
        }
    }

    pub const fn sentence(self) -> &'static str {
        match self {
            Self::Ai => "Where Sovra's answers come from.",
            Self::Chat => "Talk to Sovra from your phone.",
            Self::Accounts => "Let Sovra help with your email, calendar, documents and code.",
            Self::Storage => "Buckets and file servers Sovra can look through and save to.",
            Self::Tools => "Add-on tools that give Sovra new skills.",
        }
    }

    /// The kinds in this section, in page order.
    pub fn kinds(self) -> impl Iterator<Item = CatalogueKind> {
        CatalogueKind::ALL
            .into_iter()
            .filter(move |kind| kind.group() == self)
    }
}

/// What the catalogue row says about a link. One word-picture, shared by every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkPhase {
    #[default]
    NotLinked,
    Working,
    Linked,
    NeedsYou,
}

impl LinkPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotLinked => "Not linked",
            Self::Working => "Working on it",
            Self::Linked => "Linked",
            Self::NeedsYou => "Needs you",
        }
    }
}

/// Plain words for one shipped kind. One title, one sentence, one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCopy {
    pub kind: CatalogueKind,
    pub title: &'static str,
    pub sentence: &'static str,
    pub action: &'static str,
}

/// The closed list. Adding a kind means adding a row here and a variant above.
pub const KINDS: [KindCopy; 10] = [
    KindCopy {
        kind: CatalogueKind::OnlineAi,
        title: "An online AI",
        sentence: "Use an AI on the internet when you want Sovra to answer from there.",
        action: "Link an online AI",
    },
    KindCopy {
        kind: CatalogueKind::LocalAi,
        title: "An AI on this computer",
        sentence: "Use a free AI that stays on this computer.",
        action: "Use an AI on this computer",
    },
    KindCopy {
        kind: CatalogueKind::CompatibleService,
        title: "Another compatible service",
        sentence: "Connect a different AI service you already run.",
        action: "Add a compatible service",
    },
    KindCopy {
        kind: CatalogueKind::WhatsApp,
        title: "WhatsApp",
        sentence: "Message Sovra from your own WhatsApp after you link this computer.",
        action: "Connect WhatsApp",
    },
    KindCopy {
        kind: CatalogueKind::Telegram,
        title: "Telegram",
        sentence: "Talk to Sovra from Telegram after you link your bot.",
        action: "Connect Telegram",
    },
    KindCopy {
        kind: CatalogueKind::Google,
        title: "Google",
        sentence: "Sign in with Google in your browser and choose what Sovra may use.",
        action: "Connect Google",
    },
    KindCopy {
        kind: CatalogueKind::GitHub,
        title: "GitHub",
        sentence: "Approve Sovra on GitHub's page and it can help with your code projects there.",
        action: "Connect GitHub",
    },
    KindCopy {
        kind: CatalogueKind::Microsoft,
        title: "Microsoft 365",
        sentence: "Connect your own Outlook, OneDrive and other Microsoft services with guided setup.",
        action: "Connect Microsoft 365",
    },
    KindCopy {
        kind: CatalogueKind::Storage,
        title: "Online storage",
        sentence: "Link a storage bucket or file server, like Amazon S3, Cloudflare R2, Backblaze, Azure or Nextcloud, so Sovra can find, read and save files there.",
        action: "Add storage",
    },
    KindCopy {
        kind: CatalogueKind::WebAddress,
        title: "MCP servers",
        sentence: "Plug extra tools into Sovra, like reading your files, checking a calendar or searching a database.",
        action: "Add MCP server",
    },
];

/// One row on the first screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueCard {
    pub kind: CatalogueKind,
    pub title: &'static str,
    pub sentence: &'static str,
    pub action: &'static str,
    pub phase: LinkPhase,
}

/// The page, in [`KINDS`] order, with a phase on every shipped kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCatalogue {
    cards: Vec<CatalogueCard>,
}

impl ConnectionCatalogue {
    /// Every shipped kind, not linked yet.
    pub fn fresh() -> Self {
        Self {
            cards: KINDS
                .iter()
                .map(|copy| CatalogueCard {
                    kind: copy.kind,
                    title: copy.title,
                    sentence: copy.sentence,
                    action: copy.action,
                    phase: LinkPhase::NotLinked,
                })
                .collect(),
        }
    }

    pub fn cards(&self) -> &[CatalogueCard] {
        &self.cards
    }

    pub fn card(&self, kind: CatalogueKind) -> &CatalogueCard {
        self.cards
            .iter()
            .find(|card| card.kind == kind)
            .expect("every shipped kind is in the catalogue")
    }

    /// Fold one kind. Every other card stays as it was, including its error text's phase.
    pub fn apply_phase(&mut self, kind: CatalogueKind, phase: LinkPhase) {
        if let Some(card) = self.cards.iter_mut().find(|card| card.kind == kind) {
            card.phase = phase;
        }
    }

    /// How many kinds are linked, for the sidebar mark.
    ///
    /// `None` when nothing is linked. The number counts every linked kind, so
    /// one name cannot stand in for the rest.
    pub fn nav_summary(&self) -> Option<String> {
        let count = self
            .cards
            .iter()
            .filter(|card| card.phase == LinkPhase::Linked)
            .count();
        if count == 0 {
            None
        } else {
            Some(count.to_string())
        }
    }

    /// Titles, sentences, actions, and status words shown before a card is opened.
    pub fn first_screen_copy(&self) -> String {
        let mut text = String::new();
        for card in &self.cards {
            text.push_str(card.title);
            text.push('\n');
            text.push_str(card.sentence);
            text.push('\n');
            text.push_str(card.action);
            text.push('\n');
            text.push_str(card.phase.label());
            text.push('\n');
        }
        text
    }
}

/// `true` when first-screen copy asks for something that belongs on an opened card.
pub fn first_screen_asks_for_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    FIRST_SCREEN_FORBIDDEN
        .iter()
        .any(|word| lower.contains(word))
}

/// The pieces the page already knows, plus any probe that has reported since refresh.
#[derive(Debug)]
pub struct CatalogueSources<'a> {
    pub connections: &'a [ModelConnection],
    pub catalogs: &'a BTreeMap<String, CatalogState>,
    pub whatsapp: &'a WhatsAppStatus,
    pub telegram: &'a TelegramStatus,
    pub google: &'a GoogleState,
    pub github: &'a GitHubState,
    pub microsoft: &'a MicrosoftState,
    pub storage: &'a StorageBook,
    pub services: &'a [McpService],
    /// A probe result replaces only that kind's derived phase.
    pub probed: &'a BTreeMap<CatalogueKind, LinkPhase>,
}

/// Build the page from current state. One kind's phase never writes another kind.
pub fn catalogue_from(sources: &CatalogueSources<'_>) -> ConnectionCatalogue {
    let mut catalogue = ConnectionCatalogue::fresh();
    let derived = [
        (
            CatalogueKind::OnlineAi,
            provider_phase(
                sources.connections,
                sources.catalogs,
                ProviderKind::OpenRouter,
            ),
        ),
        (
            CatalogueKind::LocalAi,
            provider_phase(sources.connections, sources.catalogs, ProviderKind::Ollama),
        ),
        (
            CatalogueKind::CompatibleService,
            provider_phase(
                sources.connections,
                sources.catalogs,
                ProviderKind::OpenAiCompatible,
            ),
        ),
        (CatalogueKind::WhatsApp, whatsapp_phase(sources.whatsapp)),
        (CatalogueKind::Telegram, telegram_phase(sources.telegram)),
        (CatalogueKind::Google, google_phase(sources.google)),
        (CatalogueKind::GitHub, github_phase(sources.github)),
        (CatalogueKind::Microsoft, microsoft_phase(sources.microsoft)),
        (CatalogueKind::Storage, storage_phase(sources.storage)),
        (CatalogueKind::WebAddress, web_phase(sources.services)),
    ];
    for (kind, phase) in derived {
        let phase = sources.probed.get(&kind).copied().unwrap_or(phase);
        catalogue.apply_phase(kind, phase);
    }
    catalogue
}

fn provider_phase(
    connections: &[ModelConnection],
    catalogs: &BTreeMap<String, CatalogState>,
    kind: ProviderKind,
) -> LinkPhase {
    let matching: Vec<_> = connections.iter().filter(|c| c.kind == kind).collect();
    if matching.is_empty() {
        return LinkPhase::NotLinked;
    }
    let mut ready = false;
    let mut loading = false;
    let mut failed = false;
    let mut needs_key = false;
    for connection in matching {
        let catalog = catalogs.get(&connection.id);
        if catalog.is_some_and(|catalog| catalog.loading) {
            loading = true;
        }
        if catalog.is_some_and(|catalog| catalog.error.is_some()) {
            failed = true;
        }
        if connection.is_ready() {
            ready = true;
        } else {
            needs_key = true;
        }
    }
    if failed || needs_key {
        LinkPhase::NeedsYou
    } else if loading && !ready {
        LinkPhase::Working
    } else if ready {
        LinkPhase::Linked
    } else {
        LinkPhase::NotLinked
    }
}

pub fn whatsapp_phase(status: &WhatsAppStatus) -> LinkPhase {
    match status {
        WhatsAppStatus::Disconnected => LinkPhase::NotLinked,
        WhatsAppStatus::Connecting { .. }
        | WhatsAppStatus::AwaitingQrScan { .. }
        | WhatsAppStatus::VerifyingHandshake { .. }
        | WhatsAppStatus::Connected { online: false, .. } => LinkPhase::Working,
        WhatsAppStatus::Connected { online: true, .. } => LinkPhase::Linked,
        WhatsAppStatus::Error { .. } => LinkPhase::NeedsYou,
    }
}

pub fn telegram_phase(status: &TelegramStatus) -> LinkPhase {
    match status {
        TelegramStatus::Disconnected => LinkPhase::NotLinked,
        TelegramStatus::Connecting | TelegramStatus::Connected { online: false, .. } => {
            LinkPhase::Working
        }
        TelegramStatus::AwaitingOwner { .. } | TelegramStatus::Error { .. } => LinkPhase::NeedsYou,
        TelegramStatus::Connected { online: true, .. } => LinkPhase::Linked,
    }
}

pub fn google_phase(state: &GoogleState) -> LinkPhase {
    match state {
        GoogleState::Loading | GoogleState::Connecting { .. } => LinkPhase::Working,
        GoogleState::Unavailable { .. } | GoogleState::NeedsSignIn(_) => LinkPhase::NeedsYou,
        GoogleState::NotConnected => LinkPhase::NotLinked,
        GoogleState::Connected(_) => LinkPhase::Linked,
    }
}

pub fn microsoft_phase(state: &MicrosoftState) -> LinkPhase {
    match state {
        MicrosoftState::Loading | MicrosoftState::Connecting { .. } => LinkPhase::Working,
        MicrosoftState::NeedsSetup | MicrosoftState::NeedsSignIn(_) => LinkPhase::NeedsYou,
        MicrosoftState::NotConnected => LinkPhase::NotLinked,
        MicrosoftState::Connected(_) => LinkPhase::Linked,
    }
}

pub fn github_phase(state: &GitHubState) -> LinkPhase {
    match state {
        GitHubState::Loading | GitHubState::Connecting { .. } | GitHubState::CheckingKey => {
            LinkPhase::Working
        }
        GitHubState::NeedsSignIn(_) => LinkPhase::NeedsYou,
        GitHubState::NotConnected => LinkPhase::NotLinked,
        GitHubState::Connected(_) => LinkPhase::Linked,
    }
}

pub fn storage_phase(book: &StorageBook) -> LinkPhase {
    if book.places.is_empty() {
        return LinkPhase::NotLinked;
    }
    let statuses: Vec<_> = book
        .places
        .iter()
        .map(|view| book.status_of(&view.place.id))
        .collect();
    if statuses
        .iter()
        .any(|status| matches!(status, PlaceStatus::Problem(_)))
        || book.places.iter().any(|view| !view.has_secret)
    {
        LinkPhase::NeedsYou
    } else if statuses
        .iter()
        .any(|status| matches!(status, PlaceStatus::Checking))
        && !statuses
            .iter()
            .any(|status| matches!(status, PlaceStatus::Works))
    {
        LinkPhase::Working
    } else {
        LinkPhase::Linked
    }
}

pub fn web_phase(services: &[McpService]) -> LinkPhase {
    if services.is_empty() {
        return LinkPhase::NotLinked;
    }
    if services
        .iter()
        .any(|service| service.phase == McpPhase::Connected)
    {
        return LinkPhase::Linked;
    }
    if services.iter().any(|service| {
        matches!(
            service.phase,
            McpPhase::Failed | McpPhase::NeedsSignIn | McpPhase::NeedsClientDetails
        )
    }) {
        return LinkPhase::NeedsYou;
    }
    if services
        .iter()
        .any(|service| service.phase == McpPhase::Connecting)
    {
        LinkPhase::Working
    } else {
        LinkPhase::NotLinked
    }
}

/// What one installed link reported. The detail is a phase, never a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub kind: CatalogueKind,
    pub phase: LinkPhase,
}

/// A check the runtime can run on its own. Dropping the future cancels it.
///
/// `None` means the check did not contact the service. The catalogue keeps the
/// live status; a report would replace it.
#[async_trait]
pub trait LinkProbe: Send + Sync {
    fn kind(&self) -> CatalogueKind;
    async fn probe(&self) -> Option<ProbeReport>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::GoogleState;
    use crate::models::{CatalogState, ModelConnection, ProviderKind};

    static NO_STORAGE: StorageBook = StorageBook {
        loaded: false,
        places: Vec::new(),
        status: BTreeMap::new(),
        browsing: None,
        refused: None,
        saved: None,
        notice: None,
    };

    #[test]
    fn every_kind_sits_in_one_section_and_sections_are_in_page_order() {
        let mut seen = Vec::new();
        for group in CatalogueGroup::ALL {
            assert!(group.sentence().ends_with('.'));
            assert!(!first_screen_asks_for_secret(group.title()));
            assert!(!first_screen_asks_for_secret(group.sentence()));
            let kinds: Vec<_> = group.kinds().collect();
            assert!(!kinds.is_empty(), "{group:?} is empty");
            seen.extend(kinds);
        }
        assert_eq!(seen, CatalogueKind::ALL);
    }

    #[test]
    fn storage_card_follows_its_places() {
        use crate::storage::{PlaceView, StoragePlace, StorageService};
        let mut book = StorageBook::default();
        assert_eq!(storage_phase(&book), LinkPhase::NotLinked);
        let place = StoragePlace {
            id: "place-1".into(),
            ..StoragePlace::new(StorageService::CloudflareR2)
        };
        book.places.push(PlaceView {
            place,
            has_secret: true,
        });
        assert_eq!(storage_phase(&book), LinkPhase::Linked);
        book.status.insert("place-1".into(), PlaceStatus::Checking);
        assert_eq!(storage_phase(&book), LinkPhase::Working);
        book.status
            .insert("place-1".into(), PlaceStatus::Problem("keys".into()));
        assert_eq!(storage_phase(&book), LinkPhase::NeedsYou);
        book.status.insert("place-1".into(), PlaceStatus::Works);
        book.places[0].has_secret = false;
        assert_eq!(storage_phase(&book), LinkPhase::NeedsYou);
    }

    fn sources<'a>(
        connections: &'a [ModelConnection],
        catalogs: &'a BTreeMap<String, CatalogState>,
        whatsapp: &'a WhatsAppStatus,
        telegram: &'a TelegramStatus,
        google: &'a GoogleState,
        services: &'a [McpService],
        probed: &'a BTreeMap<CatalogueKind, LinkPhase>,
    ) -> CatalogueSources<'a> {
        CatalogueSources {
            connections,
            catalogs,
            whatsapp,
            telegram,
            google,
            github: &GitHubState::NotConnected,
            microsoft: &MicrosoftState::NotConnected,
            storage: &NO_STORAGE,
            services,
            probed,
        }
    }

    #[test]
    fn shipped_kinds_have_one_plain_title_sentence_and_action() {
        let catalogue = ConnectionCatalogue::fresh();
        assert_eq!(catalogue.cards().len(), CatalogueKind::ALL.len());
        assert_eq!(KINDS.len(), CatalogueKind::ALL.len());
        for (card, kind) in catalogue.cards().iter().zip(CatalogueKind::ALL) {
            let copy = KINDS.iter().find(|row| row.kind == kind).expect("kind row");
            assert_eq!(card.kind, kind);
            assert_eq!(card.title, copy.title);
            assert_eq!(card.sentence, copy.sentence);
            assert!(card.sentence.ends_with('.'));
            assert_eq!(card.action, copy.action);
            assert_eq!(card.phase, LinkPhase::NotLinked);
            assert_eq!(card.phase.label(), "Not linked");
        }
        let copy = catalogue.first_screen_copy();
        assert!(
            !first_screen_asks_for_secret(&copy),
            "first screen asked for a secret: {copy}"
        );
        for phase in [
            LinkPhase::NotLinked,
            LinkPhase::Working,
            LinkPhase::Linked,
            LinkPhase::NeedsYou,
        ] {
            assert!(!first_screen_asks_for_secret(phase.label()));
        }
    }

    #[test]
    fn microsoft_is_listed_but_only_linked_after_a_real_connection() {
        let catalogue = ConnectionCatalogue::fresh();
        assert!(catalogue.first_screen_copy().contains("Microsoft 365"));
        assert_eq!(
            microsoft_phase(&MicrosoftState::NeedsSetup),
            LinkPhase::NeedsYou
        );
        assert_eq!(
            microsoft_phase(&MicrosoftState::NotConnected),
            LinkPhase::NotLinked
        );
        assert_eq!(
            microsoft_phase(&MicrosoftState::Connected(
                crate::microsoft::MicrosoftAccount::default()
            )),
            LinkPhase::Linked
        );
    }

    #[test]
    fn github_card_follows_the_github_connection() {
        let account = crate::github::GitHubAccount {
            login: "octocat".into(),
            name: String::new(),
        };
        for (state, phase) in [
            (GitHubState::Loading, LinkPhase::Working),
            (GitHubState::NotConnected, LinkPhase::NotLinked),
            (GitHubState::CheckingKey, LinkPhase::Working),
            (GitHubState::Connected(account.clone()), LinkPhase::Linked),
            (GitHubState::NeedsSignIn(account), LinkPhase::NeedsYou),
        ] {
            let catalogue = catalogue_from(&CatalogueSources {
                connections: &[],
                catalogs: &BTreeMap::new(),
                whatsapp: &WhatsAppStatus::Disconnected,
                telegram: &TelegramStatus::Disconnected,
                google: &GoogleState::NotConnected,
                github: &state,
                microsoft: &MicrosoftState::NotConnected,
                storage: &StorageBook::default(),
                services: &[],
                probed: &BTreeMap::new(),
            });
            assert_eq!(
                catalogue.card(CatalogueKind::GitHub).phase,
                phase,
                "{state:?}"
            );
            assert_eq!(
                catalogue.card(CatalogueKind::Google).phase,
                LinkPhase::NotLinked
            );
        }
    }

    #[test]
    fn one_kind_error_leaves_every_other_kind_unchanged() {
        let mut connections = vec![ModelConnection::new("router", ProviderKind::OpenRouter)];
        connections[0].has_credential = true;
        let mut ollama = ModelConnection::new("local", ProviderKind::Ollama);
        ollama.base_url = ProviderKind::Ollama.default_url().into();
        connections.push(ollama);
        let catalogs = BTreeMap::new();
        let whatsapp = WhatsAppStatus::Error {
            message: "The phone could not be reached.".into(),
        };
        let telegram = TelegramStatus::Connected {
            bot_username: "sovra_bot".into(),
            owner_chat_id: "1".into(),
            owner_name: "Sam".into(),
            linked_at: crate::memory::Timestamp(1),
            online: true,
        };
        let google = GoogleState::NeedsSignIn(crate::google::GoogleAccount {
            email: "sam@example.com".into(),
            name: "Sam".into(),
            granted: std::collections::BTreeSet::new(),
            turned_off: std::collections::BTreeSet::new(),
        });
        let services = vec![McpService {
            id: "recipes".into(),
            name: "Recipes".into(),
            address: "https://recipes.example.com".into(),
            phase: McpPhase::Failed,
            message: "Could not reach it.".into(),
            tools: Vec::new(),
            ..McpService::default()
        }];
        let probed = BTreeMap::new();
        let catalogue = catalogue_from(&sources(
            &connections,
            &catalogs,
            &whatsapp,
            &telegram,
            &google,
            &services,
            &probed,
        ));
        assert_eq!(
            catalogue.card(CatalogueKind::WhatsApp).phase,
            LinkPhase::NeedsYou
        );
        assert_eq!(
            catalogue.card(CatalogueKind::Telegram).phase,
            LinkPhase::Linked
        );
        assert_eq!(
            catalogue.card(CatalogueKind::OnlineAi).phase,
            LinkPhase::Linked
        );
        assert_eq!(
            catalogue.card(CatalogueKind::LocalAi).phase,
            LinkPhase::Linked
        );
        assert_eq!(
            catalogue.card(CatalogueKind::Google).phase,
            LinkPhase::NeedsYou
        );
        assert_eq!(
            catalogue.card(CatalogueKind::WebAddress).phase,
            LinkPhase::NeedsYou
        );
        assert_eq!(
            catalogue.card(CatalogueKind::CompatibleService).phase,
            LinkPhase::NotLinked
        );
        assert_eq!(
            catalogue.card(CatalogueKind::GitHub).phase,
            LinkPhase::NotLinked
        );

        let before: Vec<_> = catalogue
            .cards()
            .iter()
            .map(|card| (card.kind, card.phase))
            .collect();
        let mut next = catalogue;
        next.apply_phase(CatalogueKind::WhatsApp, LinkPhase::Working);
        for (kind, phase) in before {
            if kind == CatalogueKind::WhatsApp {
                assert_eq!(next.card(kind).phase, LinkPhase::Working);
            } else {
                assert_eq!(next.card(kind).phase, phase);
            }
        }

        let mut probed = BTreeMap::new();
        probed.insert(CatalogueKind::Telegram, LinkPhase::NeedsYou);
        let overridden = catalogue_from(&sources(
            &connections,
            &catalogs,
            &WhatsAppStatus::Disconnected,
            &telegram,
            &GoogleState::NotConnected,
            &[],
            &probed,
        ));
        assert_eq!(
            overridden.card(CatalogueKind::Telegram).phase,
            LinkPhase::NeedsYou
        );
        assert_eq!(
            overridden.card(CatalogueKind::WhatsApp).phase,
            LinkPhase::NotLinked
        );
        assert_eq!(
            overridden.card(CatalogueKind::OnlineAi).phase,
            LinkPhase::Linked
        );
    }

    #[test]
    fn nav_summary_counts_every_linked_kind() {
        assert_eq!(ConnectionCatalogue::fresh().nav_summary(), None);

        let connecting = WhatsAppStatus::Connecting { pairing: false };
        let working = catalogue_from(&sources(
            &[],
            &BTreeMap::new(),
            &connecting,
            &TelegramStatus::Disconnected,
            &GoogleState::NotConnected,
            &[],
            &BTreeMap::new(),
        ));
        assert_eq!(working.nav_summary(), None);

        let whatsapp = WhatsAppStatus::Connected {
            device: crate::connections::LinkedDevice {
                phone_display: "+1 555-0142".into(),
                device_jid: "15550142:1@s.whatsapp.net".into(),
                companion_name: "Sovra".into(),
                linked_at: crate::memory::Timestamp(10_000),
            },
            online: true,
        };
        let only_whatsapp = catalogue_from(&sources(
            &[],
            &BTreeMap::new(),
            &whatsapp,
            &TelegramStatus::Disconnected,
            &GoogleState::NotConnected,
            &[],
            &BTreeMap::new(),
        ));
        assert_eq!(only_whatsapp.nav_summary().as_deref(), Some("1"));

        let google = GoogleState::Connected(crate::google::GoogleAccount::default());
        let services = vec![McpService {
            id: "files".into(),
            name: "Files".into(),
            address: "https://files.example".into(),
            phase: McpPhase::Connected,
            message: String::new(),
            tools: Vec::new(),
            ..McpService::default()
        }];
        let several = catalogue_from(&CatalogueSources {
            connections: &[],
            catalogs: &BTreeMap::new(),
            whatsapp: &whatsapp,
            telegram: &TelegramStatus::Disconnected,
            google: &google,
            github: &GitHubState::NotConnected,
            microsoft: &MicrosoftState::NotConnected,
            storage: &StorageBook::default(),
            services: &services,
            probed: &BTreeMap::new(),
        });
        assert_eq!(several.nav_summary().as_deref(), Some("3"));
        assert_eq!(
            several.card(CatalogueKind::Telegram).phase,
            LinkPhase::NotLinked
        );
    }
}
