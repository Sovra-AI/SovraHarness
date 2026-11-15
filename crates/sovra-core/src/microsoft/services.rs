//! One source for service labels, requested scopes and effective grants.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicrosoftService {
    Mail,
    Calendar,
    Files,
    Contacts,
    Tasks,
    Notes,
    SharePoint,
    Chats,
    Channels,
    Planner,
}

#[derive(Debug, Clone, Copy)]
pub struct ServiceInfo {
    pub service: MicrosoftService,
    pub label: &'static str,
    pub explanation: &'static str,
    pub scopes: &'static [&'static str],
    pub organization_only: bool,
    pub admin_consent: bool,
}

pub const SERVICES: [ServiceInfo; 10] = [
    ServiceInfo {
        service: MicrosoftService::Mail,
        label: "Outlook email",
        explanation: "Read email and prepare drafts for you to review and send.",
        scopes: &["Mail.ReadWrite"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Calendar,
        label: "Outlook calendar",
        explanation: "See your plans and add appointments without inviting anyone.",
        scopes: &["Calendars.ReadWrite"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Files,
        label: "OneDrive files",
        explanation: "Find and read documents, spreadsheets and presentations.",
        scopes: &["Files.Read"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Contacts,
        label: "Outlook contacts",
        explanation: "Look up names, email addresses and phone numbers.",
        scopes: &["Contacts.Read"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Tasks,
        label: "Microsoft To Do",
        explanation: "Read your lists and add to-dos when you ask.",
        scopes: &["Tasks.ReadWrite"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Notes,
        label: "OneNote",
        explanation: "Find and read your notebook pages.",
        scopes: &["Notes.Read"],
        organization_only: false,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::SharePoint,
        label: "SharePoint",
        explanation: "Find sites and read shared documents you can access at work.",
        scopes: &["Sites.Read.All"],
        organization_only: true,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Chats,
        label: "Teams chats",
        explanation: "Read the work conversations you belong to.",
        scopes: &["Chat.Read"],
        organization_only: true,
        admin_consent: false,
    },
    ServiceInfo {
        service: MicrosoftService::Channels,
        label: "Teams channels",
        explanation: "Read team discussions. Your organization must approve this.",
        scopes: &[
            "Team.ReadBasic.All",
            "Channel.ReadBasic.All",
            "ChannelMessage.Read.All",
        ],
        organization_only: true,
        admin_consent: true,
    },
    ServiceInfo {
        service: MicrosoftService::Planner,
        label: "Planner",
        explanation: "See the work tasks assigned to you.",
        scopes: &["Tasks.Read"],
        organization_only: true,
        admin_consent: false,
    },
];

impl MicrosoftService {
    pub fn info(self) -> &'static ServiceInfo {
        SERVICES
            .iter()
            .find(|info| info.service == self)
            .expect("every service has metadata")
    }

    /// Consumer-compatible services; work-only grants are requested when enabled.
    pub fn defaults() -> BTreeSet<Self> {
        SERVICES
            .iter()
            .filter(|info| !info.organization_only)
            .map(|info| info.service)
            .collect()
    }
}

pub fn scopes_for(services: &BTreeSet<MicrosoftService>) -> Vec<&'static str> {
    let mut scopes = vec!["User.Read", "offline_access"];
    for service in services {
        for scope in service.info().scopes {
            if !scopes.contains(scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

/// Only returned scopes count. Full Graph scope URIs and short names are equivalent.
pub fn granted_services(scope: &str) -> BTreeSet<MicrosoftService> {
    let grants: BTreeSet<_> = scope
        .split_whitespace()
        .map(|scope| {
            scope
                .strip_prefix("https://graph.microsoft.com/")
                .unwrap_or(scope)
        })
        .collect();
    SERVICES
        .iter()
        .filter(|info| {
            info.scopes.iter().all(|scope| {
                grants.contains(scope)
                    || match *scope {
                        "Files.Read" => {
                            grants.contains("Files.Read.All")
                                || grants.contains("Files.ReadWrite")
                                || grants.contains("Files.ReadWrite.All")
                        }
                        "Tasks.Read" => grants.contains("Tasks.ReadWrite"),
                        "Contacts.Read" => grants.contains("Contacts.ReadWrite"),
                        "Notes.Read" => {
                            grants.contains("Notes.ReadWrite")
                                || grants.contains("Notes.Read.All")
                                || grants.contains("Notes.ReadWrite.All")
                        }
                        "Chat.Read" => grants.contains("Chat.ReadWrite"),
                        "Sites.Read.All" => grants.contains("Sites.ReadWrite.All"),
                        _ => false,
                    }
            })
        })
        .map(|info| info.service)
        .collect()
}
