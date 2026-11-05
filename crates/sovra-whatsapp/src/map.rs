//! Pure mapping from `WhatsApp` Web address-book rows and participating-group
//! metadata into core contacts and groups. Groups are never people.

use sovra_core::connections::{SyncedGroup, is_whatsapp_group_id};
use sovra_core::people::{SyncedContact, phone_digits};

/// Address-book fields the phone actually sends (app-state contact action or
/// history conversation metadata). Message bodies are not used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContactRow<'a> {
    pub jid: &'a str,
    pub pn_jid: Option<&'a str>,
    pub full_name: Option<&'a str>,
    pub first_name: Option<&'a str>,
    pub save_on_primary_addressbook: Option<bool>,
}

/// Conversation metadata from history bootstrap. Message bodies are ignored.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoryChat<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub pn_jid: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MappedChat {
    Contact(SyncedContact),
    Group(SyncedGroup),
    Skip,
}

pub(crate) fn map_contact_row(row: ContactRow<'_>) -> Option<SyncedContact> {
    if is_skipped_chat(row.jid) || is_whatsapp_group_id(row.jid) {
        return None;
    }
    let phone = phone_digits(row.jid).or_else(|| row.pn_jid.and_then(phone_digits))?;
    let name = pick_name(row.full_name, row.first_name);
    let removed = name.is_empty() && row.save_on_primary_addressbook == Some(false);
    Some(SyncedContact {
        name,
        phone,
        removed,
    })
}

pub(crate) fn map_group(id: &str, subject: &str) -> Option<SyncedGroup> {
    if !is_whatsapp_group_id(id) {
        return None;
    }
    Some(SyncedGroup {
        id: id.to_owned(),
        subject: clean_name(subject),
    })
}

pub(crate) fn map_history_chat(chat: HistoryChat<'_>) -> MappedChat {
    if is_skipped_chat(chat.id) {
        return MappedChat::Skip;
    }
    if is_whatsapp_group_id(chat.id) {
        return map_group(chat.id, chat.name.or(chat.display_name).unwrap_or(""))
            .map_or(MappedChat::Skip, MappedChat::Group);
    }
    map_contact_row(ContactRow {
        jid: chat.id,
        pn_jid: chat.pn_jid,
        full_name: chat.name.or(chat.display_name),
        first_name: chat.display_name,
        save_on_primary_addressbook: None,
    })
    .map_or(MappedChat::Skip, MappedChat::Contact)
}

fn is_skipped_chat(id: &str) -> bool {
    let id = id.trim();
    id.is_empty()
        || id.starts_with("0@")
        || id.ends_with("@broadcast")
        || id.ends_with("@newsletter")
}

fn pick_name(full: Option<&str>, first: Option<&str>) -> String {
    let full = full.map(clean_name).filter(|name| !name.is_empty());
    full.or_else(|| first.map(clean_name).filter(|name| !name.is_empty()))
        .unwrap_or_default()
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_number_contact_becomes_a_person() {
        let contact = map_contact_row(ContactRow {
            jid: "447700900111@s.whatsapp.net",
            pn_jid: None,
            full_name: Some("Sam Rivera"),
            first_name: Some("Sam"),
            save_on_primary_addressbook: Some(true),
        })
        .expect("phone contact");
        assert_eq!(contact.phone, "447700900111");
        assert_eq!(contact.name, "Sam Rivera");
        assert!(!contact.removed);
    }

    #[test]
    fn a_lid_contact_with_a_phone_number_becomes_a_person() {
        let contact = map_contact_row(ContactRow {
            jid: "123456789012345@lid",
            pn_jid: Some("447700900222@s.whatsapp.net"),
            full_name: Some("Ada Lovelace"),
            first_name: None,
            save_on_primary_addressbook: Some(true),
        })
        .expect("lid with phone");
        assert_eq!(contact.phone, "447700900222");
        assert_eq!(contact.name, "Ada Lovelace");
    }

    #[test]
    fn a_contact_with_no_usable_phone_is_not_a_person() {
        assert!(
            map_contact_row(ContactRow {
                jid: "123456789012345@lid",
                pn_jid: None,
                full_name: Some("Hidden"),
                first_name: None,
                save_on_primary_addressbook: Some(true),
            })
            .is_none()
        );
        assert!(
            map_contact_row(ContactRow {
                jid: "120363111@g.us",
                pn_jid: Some("447700900111@s.whatsapp.net"),
                full_name: Some("School Gate"),
                first_name: None,
                save_on_primary_addressbook: Some(true),
            })
            .is_none()
        );
    }

    #[test]
    fn a_participating_group_is_stored_as_a_group_not_a_person() {
        let group = map_group("120363111@g.us", "  School   Gate  ").expect("group");
        assert_eq!(group.id, "120363111@g.us");
        assert_eq!(group.subject, "School Gate");
        assert!(map_group("447700900111@s.whatsapp.net", "Sam").is_none());
        match map_history_chat(HistoryChat {
            id: "120363111@g.us",
            name: Some("School Gate"),
            display_name: None,
            pn_jid: None,
        }) {
            MappedChat::Group(mapped) => assert_eq!(mapped.subject, "School Gate"),
            other => panic!("group must not become {other:?}"),
        }
        match map_history_chat(HistoryChat {
            id: "123456789012345@lid",
            name: Some("Ada Lovelace"),
            display_name: None,
            pn_jid: Some("447700900222"),
        }) {
            MappedChat::Contact(contact) => {
                assert_eq!(contact.phone, "447700900222");
                assert_eq!(contact.name, "Ada Lovelace");
            }
            other => panic!("lid-with-phone must be a person, got {other:?}"),
        }
        assert_eq!(
            map_history_chat(HistoryChat {
                id: "status@broadcast",
                name: Some("Status"),
                display_name: None,
                pn_jid: None,
            }),
            MappedChat::Skip
        );
    }

    #[test]
    fn history_sync_is_processed_not_acknowledged_and_discarded() {
        let src = include_str!("lib.rs");
        assert!(
            !src.contains("skip_history_sync"),
            "ack-and-discard of INITIAL_BOOTSTRAP drops contacts and groups"
        );
    }
}
