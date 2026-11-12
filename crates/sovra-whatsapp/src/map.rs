//! Pure mapping from `WhatsApp` Web address-book rows, history chunks, and
//! chat-metadata ids into core contacts and groups.
//!
//! A chat that is not saved on the phone is still a person. Groups are never
//! people. A phone number is recorded only when this same sync names it.

use std::collections::{BTreeMap, HashMap};

use sovra_core::connections::{
    AddressBookSnapshot, ChannelEvent, SyncedGroup, is_whatsapp_group_id,
};
use sovra_core::people::{SyncedContact, phone_digits};

/// Address-book fields the phone actually sends (app-state contact action or
/// an inline history contact). Message bodies are not used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContactRow<'a> {
    pub jid: &'a str,
    pub pn_jid: Option<&'a str>,
    pub full_name: Option<&'a str>,
    pub first_name: Option<&'a str>,
    pub save_on_primary_addressbook: Option<bool>,
    /// The phone dropped this address-book row. "Not saved on the phone" is not
    /// this flag: an unsaved chat stays.
    pub removed: bool,
}

/// Conversation metadata from one history chunk. Message bodies are ignored.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoryChat<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub pn_jid: Option<&'a str>,
}

/// One sync the phone delivered. Hidden ids resolve only from pairs in this
/// same batch (a row's own phone, or the chunk's phone↔hidden-id map).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyncBatch<'a> {
    pub contacts: &'a [ContactRow<'a>],
    pub chats: &'a [HistoryChat<'a>],
    /// `(id, push name)` from the same history chunk.
    pub push_names: &'a [(&'a str, &'a str)],
    /// `(hidden id, phone jid)` from the same history chunk.
    pub phone_by_hidden_id: &'a [(&'a str, &'a str)],
    /// Chat ids named by pin, archive, mute, or mark-read. Not deletions.
    pub named_chat_ids: &'a [&'a str],
    /// Explicit address-book removals. Drops that number only.
    pub removed_ids: &'a [&'a str],
    /// Delete-chat ids. Drops that chat only, not a saved address-book person
    /// named in this same batch.
    pub deleted_chat_ids: &'a [&'a str],
}

impl SyncBatch<'static> {
    pub(crate) fn empty() -> Self {
        Self {
            contacts: &[],
            chats: &[],
            push_names: &[],
            phone_by_hidden_id: &[],
            named_chat_ids: &[],
            removed_ids: &[],
            deleted_chat_ids: &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MappedChat {
    Contact(SyncedContact),
    Group(SyncedGroup),
    Skip,
}

/// People and groups to keep from one sync, plus the single rows it drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MappedSync {
    pub contacts: Vec<SyncedContact>,
    /// Phones this batch saved on the phone's address book.
    pub saved_phones: Vec<String>,
    pub groups: Vec<SyncedGroup>,
    pub explicit_removals: Vec<String>,
    /// One-to-one chats deleted in this batch that are not saved contacts here.
    pub deleted_chats: Vec<String>,
    pub deleted_group_ids: Vec<String>,
}

pub(crate) fn map_contact_row(
    row: ContactRow<'_>,
    phones: &HashMap<String, String>,
) -> Option<SyncedContact> {
    let phone = lookup_phone(row.jid, row.pn_jid, phones)?;
    let name = pick_name(row.full_name, row.first_name);
    Some(SyncedContact {
        name,
        phone,
        removed: row.removed,
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

pub(crate) fn map_history_chat(
    chat: HistoryChat<'_>,
    phones: &HashMap<String, String>,
) -> MappedChat {
    if is_skipped_chat(chat.id) {
        return MappedChat::Skip;
    }
    if is_whatsapp_group_id(chat.id) {
        return map_group(chat.id, chat.name.or(chat.display_name).unwrap_or(""))
            .map_or(MappedChat::Skip, MappedChat::Group);
    }
    map_contact_row(
        ContactRow {
            jid: chat.id,
            pn_jid: chat.pn_jid,
            full_name: chat.name.or(chat.display_name),
            first_name: chat.display_name,
            save_on_primary_addressbook: None,
            removed: false,
        },
        phones,
    )
    .map_or(MappedChat::Skip, MappedChat::Contact)
}

/// The mapping the live link uses for contact updates, history chunks, and
/// pin / archive / mute / mark-read / delete-chat ids.
pub(crate) fn map_sync(batch: SyncBatch<'_>) -> MappedSync {
    let phones = phone_index(batch);
    let mut people: HashMap<String, PersonAccum> = HashMap::new();
    let mut groups: HashMap<String, SyncedGroup> = HashMap::new();
    let mut explicit_removals = Vec::new();
    let mut deleted_chats = Vec::new();
    let mut deleted_group_ids = Vec::new();

    for row in batch.contacts {
        if row.removed {
            if let Some(phone) = lookup_phone(row.jid, row.pn_jid, &phones) {
                explicit_removals.push(phone);
            }
            continue;
        }
        remember_contact(
            &mut people,
            map_contact_row(*row, &phones),
            row.save_on_primary_addressbook == Some(true),
        );
    }
    for chat in batch.chats {
        match map_history_chat(*chat, &phones) {
            MappedChat::Contact(contact) => remember_contact(&mut people, Some(contact), false),
            MappedChat::Group(group) => remember_group(&mut groups, group),
            MappedChat::Skip => {}
        }
    }
    for (id, push_name) in batch.push_names {
        match map_history_chat(
            HistoryChat {
                id,
                name: Some(push_name),
                display_name: None,
                pn_jid: None,
            },
            &phones,
        ) {
            MappedChat::Contact(contact) => {
                remember_contact(&mut people, Some(contact), false);
            }
            MappedChat::Group(_) | MappedChat::Skip => {}
        }
    }
    for id in batch.named_chat_ids {
        match map_history_chat(
            HistoryChat {
                id,
                name: None,
                display_name: None,
                pn_jid: None,
            },
            &phones,
        ) {
            MappedChat::Contact(contact) => {
                remember_contact(&mut people, Some(contact), false);
            }
            MappedChat::Group(group) => remember_group(&mut groups, group),
            MappedChat::Skip => {}
        }
    }
    for id in batch.removed_ids {
        if let Some(phone) = lookup_phone(id, None, &phones) {
            explicit_removals.push(phone);
        }
    }
    for phone in &explicit_removals {
        people.remove(phone);
    }
    for id in batch.deleted_chat_ids {
        if is_skipped_chat(id) {
            continue;
        }
        if is_whatsapp_group_id(id) {
            deleted_group_ids.push((*id).to_owned());
            groups.remove(*id);
            continue;
        }
        if let Some(phone) = lookup_phone(id, None, &phones) {
            // A saved address-book row in this same sync is not the chat.
            if people
                .get(&phone)
                .is_some_and(|person| person.saved_on_phone)
            {
                continue;
            }
            people.remove(&phone);
            deleted_chats.push(phone);
        }
    }

    let mut contacts: Vec<_> = people
        .into_iter()
        .map(|(phone, person)| SyncedContact {
            name: person.name,
            phone,
            removed: false,
        })
        .collect();
    contacts.sort_by(|left, right| left.phone.cmp(&right.phone));
    let mut saved_phones: Vec<_> = contacts
        .iter()
        .filter(|contact| {
            batch.contacts.iter().any(|row| {
                row.save_on_primary_addressbook == Some(true)
                    && lookup_phone(row.jid, row.pn_jid, &phones).as_deref()
                        == Some(contact.phone.as_str())
            })
        })
        .map(|contact| contact.phone.clone())
        .collect();
    saved_phones.sort();
    explicit_removals.sort();
    explicit_removals.dedup();
    deleted_chats.sort();
    deleted_chats.dedup();
    deleted_group_ids.sort();
    deleted_group_ids.dedup();
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    MappedSync {
        contacts,
        saved_phones,
        groups,
        explicit_removals,
        deleted_chats,
        deleted_group_ids,
    }
}

/// Contacts already learned, plus whatever the group query returned.
/// A group-list failure keeps the contacts and does not invent groups.
pub(crate) fn snapshot_keeping_contacts(
    contacts: Vec<SyncedContact>,
    groups: Result<Vec<SyncedGroup>, ()>,
) -> AddressBookSnapshot {
    AddressBookSnapshot {
        contacts,
        groups: groups.unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersonAccum {
    name: String,
    saved_on_phone: bool,
    name_is_saved: bool,
}

fn remember_contact(
    people: &mut HashMap<String, PersonAccum>,
    contact: Option<SyncedContact>,
    saved_on_phone: bool,
) {
    let Some(contact) = contact.filter(|contact| !contact.removed) else {
        return;
    };
    let entry = people.entry(contact.phone).or_insert_with(|| PersonAccum {
        name: String::new(),
        saved_on_phone: false,
        name_is_saved: false,
    });
    // A saved address-book name replaces a push name. A push name fills a blank.
    let fills_blank = entry.name.is_empty();
    let saved_name = saved_on_phone && !entry.name_is_saved;
    if !contact.name.is_empty() && (fills_blank || saved_name) {
        entry.name = contact.name;
        entry.name_is_saved = saved_on_phone;
    }
    entry.saved_on_phone |= saved_on_phone;
}

fn remember_group(groups: &mut HashMap<String, SyncedGroup>, group: SyncedGroup) {
    match groups.get_mut(&group.id) {
        Some(saved) if saved.subject.is_empty() && !group.subject.is_empty() => {
            saved.subject = group.subject;
        }
        Some(_) => {}
        None => {
            groups.insert(group.id.clone(), group);
        }
    }
}

fn phone_index(batch: SyncBatch<'_>) -> HashMap<String, String> {
    let mut phones = HashMap::new();
    for (hidden, phone) in batch.phone_by_hidden_id {
        note_pair(&mut phones, hidden, phone);
    }
    for row in batch.contacts {
        if let (Some(hidden), Some(phone)) = (lid_id(row.jid), row.pn_jid) {
            note_pair(&mut phones, hidden, phone);
        }
    }
    for chat in batch.chats {
        if let (Some(hidden), Some(phone)) = (lid_id(chat.id), chat.pn_jid) {
            note_pair(&mut phones, hidden, phone);
        }
    }
    phones
}

fn note_pair(phones: &mut HashMap<String, String>, hidden: &str, phone_jid: &str) {
    let Some(phone) = phone_from_jid(phone_jid) else {
        return;
    };
    let hidden = hidden.trim();
    if hidden.is_empty() || is_skipped_chat(hidden) || is_whatsapp_group_id(hidden) {
        return;
    }
    phones.insert(hidden.to_owned(), phone.clone());
    if let Some((user, _)) = hidden.split_once('@') {
        phones.insert(user.to_owned(), phone);
    }
}

fn lid_id(id: &str) -> Option<&str> {
    let server = id.split_once('@')?.1.split(':').next().unwrap_or("");
    (server == "lid").then_some(id)
}

fn lookup_phone(
    id: &str,
    pn_jid: Option<&str>,
    phones: &HashMap<String, String>,
) -> Option<String> {
    if is_skipped_chat(id) || is_whatsapp_group_id(id) {
        return None;
    }
    if let Some(phone) = phone_from_jid(id) {
        return Some(phone);
    }
    if let Some(phone) = pn_jid.and_then(phone_from_jid) {
        return Some(phone);
    }
    if let Some(phone) = phones.get(id) {
        return Some(phone.clone());
    }
    let user = id.split_once('@').map_or(id, |(user, _)| user);
    phones.get(user).cloned()
}

/// Digits from a phone jid. `@c.us` is the legacy phone server. Other servers
/// (hidden ids, groups, newsletters) are not phones.
fn phone_from_jid(jid: &str) -> Option<String> {
    let trimmed = jid.trim();
    let normalized = if let Some((user, server)) = trimmed.split_once('@') {
        let server = server.split(':').next().unwrap_or(server);
        if server == "c.us" {
            format!("{user}@s.whatsapp.net")
        } else if server == "s.whatsapp.net" {
            trimmed.to_owned()
        } else {
            return None;
        }
    } else {
        trimmed.to_owned()
    };
    phone_digits(&normalized)
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

/// Contacts and groups learned from the events this link has already seen.
/// The online pull returns this book. An empty pull must not be the only copy.
#[derive(Debug, Default)]
pub(crate) struct LearnedBook {
    people: BTreeMap<String, LearnedPerson>,
    groups: BTreeMap<String, SyncedGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LearnedPerson {
    name: String,
    saved_on_phone: bool,
}

impl LearnedBook {
    pub(crate) fn publish(&mut self, mapped: MappedSync) -> Vec<ChannelEvent> {
        let saved: std::collections::BTreeSet<_> = mapped.saved_phones.iter().cloned().collect();
        let mut contacts = Vec::new();
        for contact in mapped.contacts {
            let phone = contact.phone.clone();
            let saved_on_phone = saved.contains(&phone);
            let stored = self.upsert(contact, saved_on_phone);
            contacts.push(stored);
        }
        for phone in mapped.explicit_removals {
            self.people.remove(&phone);
            contacts.push(SyncedContact {
                name: String::new(),
                phone,
                removed: true,
            });
        }
        for phone in mapped.deleted_chats {
            if self
                .people
                .get(&phone)
                .is_some_and(|person| person.saved_on_phone)
            {
                continue;
            }
            self.people.remove(&phone);
            contacts.push(SyncedContact {
                name: String::new(),
                phone,
                removed: true,
            });
        }
        for id in &mapped.deleted_group_ids {
            self.groups.remove(id);
        }
        let groups: Vec<_> = mapped
            .groups
            .into_iter()
            .inspect(|group| {
                self.groups.insert(group.id.clone(), group.clone());
            })
            .collect();
        let mut events = Vec::new();
        if !contacts.is_empty() {
            events.push(ChannelEvent::Contacts(contacts));
        }
        if !groups.is_empty() {
            events.push(ChannelEvent::Groups(groups));
        }
        events
    }

    pub(crate) fn snapshot_contacts(&self) -> Vec<SyncedContact> {
        self.people
            .iter()
            .map(|(phone, person)| SyncedContact {
                name: person.name.clone(),
                phone: phone.clone(),
                removed: false,
            })
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.people.clear();
        self.groups.clear();
    }

    fn upsert(&mut self, contact: SyncedContact, saved_on_phone: bool) -> SyncedContact {
        let entry = self
            .people
            .entry(contact.phone.clone())
            .or_insert_with(|| LearnedPerson {
                name: String::new(),
                saved_on_phone: false,
            });
        if !contact.name.is_empty() && (entry.name.is_empty() || saved_on_phone) {
            entry.name.clone_from(&contact.name);
        }
        entry.saved_on_phone |= saved_on_phone;
        SyncedContact {
            name: entry.name.clone(),
            phone: contact.phone,
            removed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(
        jid: &'a str,
        pn_jid: Option<&'a str>,
        full_name: Option<&'a str>,
        saved: Option<bool>,
        removed: bool,
    ) -> ContactRow<'a> {
        ContactRow {
            jid,
            pn_jid,
            full_name,
            first_name: None,
            save_on_primary_addressbook: saved,
            removed,
        }
    }

    fn phone_of<'a>(contacts: &'a [SyncedContact], phone: &str) -> &'a SyncedContact {
        contacts
            .iter()
            .find(|contact| contact.phone == phone)
            .unwrap_or_else(|| panic!("missing {phone}"))
    }

    #[test]
    fn a_phone_number_contact_becomes_a_person() {
        let contact = map_contact_row(
            row(
                "447700900111@s.whatsapp.net",
                None,
                Some("Sam Rivera"),
                Some(true),
                false,
            ),
            &HashMap::new(),
        )
        .expect("phone contact");
        assert_eq!(contact.phone, "447700900111");
        assert_eq!(contact.name, "Sam Rivera");
        assert!(!contact.removed);
    }

    #[test]
    fn a_lid_contact_with_a_phone_number_becomes_a_person() {
        let contact = map_contact_row(
            row(
                "123456789012345@lid",
                Some("447700900222@s.whatsapp.net"),
                Some("Ada Lovelace"),
                Some(true),
                false,
            ),
            &HashMap::new(),
        )
        .expect("lid with phone");
        assert_eq!(contact.phone, "447700900222");
        assert_eq!(contact.name, "Ada Lovelace");
    }

    #[test]
    fn a_contact_with_no_usable_phone_is_not_a_person() {
        assert!(
            map_contact_row(
                row(
                    "123456789012345@lid",
                    None,
                    Some("Hidden"),
                    Some(true),
                    false,
                ),
                &HashMap::new(),
            )
            .is_none()
        );
        assert!(
            map_contact_row(
                row(
                    "120363111@g.us",
                    Some("447700900111@s.whatsapp.net"),
                    Some("School Gate"),
                    Some(true),
                    false,
                ),
                &HashMap::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn an_unsaved_contact_with_no_name_is_kept() {
        let contact = map_contact_row(
            row(
                "447700900555@s.whatsapp.net",
                None,
                Some(""),
                Some(false),
                false,
            ),
            &HashMap::new(),
        )
        .expect("unsaved chat");
        assert_eq!(contact.phone, "447700900555");
        assert_eq!(contact.name, "");
        assert!(!contact.removed);
    }

    #[test]
    fn a_participating_group_is_stored_as_a_group_not_a_person() {
        let group = map_group("120363111@g.us", "  School   Gate  ").expect("group");
        assert_eq!(group.id, "120363111@g.us");
        assert_eq!(group.subject, "School Gate");
        assert!(map_group("447700900111@s.whatsapp.net", "Sam").is_none());
        match map_history_chat(
            HistoryChat {
                id: "120363111@g.us",
                name: Some("School Gate"),
                display_name: None,
                pn_jid: None,
            },
            &HashMap::new(),
        ) {
            MappedChat::Group(mapped) => assert_eq!(mapped.subject, "School Gate"),
            other => panic!("group must not become {other:?}"),
        }
        match map_history_chat(
            HistoryChat {
                id: "123456789012345@lid",
                name: Some("Ada Lovelace"),
                display_name: None,
                pn_jid: Some("447700900222"),
            },
            &HashMap::new(),
        ) {
            MappedChat::Contact(contact) => {
                assert_eq!(contact.phone, "447700900222");
                assert_eq!(contact.name, "Ada Lovelace");
            }
            other => panic!("lid-with-phone must be a person, got {other:?}"),
        }
        assert_eq!(
            map_history_chat(
                HistoryChat {
                    id: "status@broadcast",
                    name: Some("Status"),
                    display_name: None,
                    pn_jid: None,
                },
                &HashMap::new(),
            ),
            MappedChat::Skip
        );
    }

    #[test]
    fn one_sync_keeps_every_named_person_and_group_and_drops_only_explicit_rows() {
        let mapped = map_sync(SyncBatch {
            contacts: &[
                row(
                    "447700900111@s.whatsapp.net",
                    None,
                    Some("Sam Rivera"),
                    Some(true),
                    false,
                ),
                row(
                    "447700900444@s.whatsapp.net",
                    None,
                    Some(""),
                    Some(false),
                    true,
                ),
            ],
            chats: &[
                HistoryChat {
                    id: "123456789012345@lid",
                    name: Some("Ada Lovelace"),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "999999999999999@lid",
                    name: Some("Ghost"),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "447700900555@s.whatsapp.net",
                    name: Some(""),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "120363111@g.us",
                    name: Some("School Gate"),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "120363222@g.us",
                    name: Some("Old Group"),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "status@broadcast",
                    name: Some("Status"),
                    display_name: None,
                    pn_jid: None,
                },
                HistoryChat {
                    id: "120363333@newsletter",
                    name: Some("News"),
                    display_name: None,
                    pn_jid: None,
                },
            ],
            push_names: &[("447700900666@s.whatsapp.net", "Pat Lee")],
            phone_by_hidden_id: &[("123456789012345@lid", "447700900222@s.whatsapp.net")],
            named_chat_ids: &[
                "447700900777@s.whatsapp.net",
                "14155550199@broadcast",
                "120363444@newsletter",
            ],
            removed_ids: &[],
            deleted_chat_ids: &[
                "447700900111@s.whatsapp.net",
                "447700900888@s.whatsapp.net",
                "120363222@g.us",
            ],
        });

        let sam = phone_of(&mapped.contacts, "447700900111");
        assert_eq!(sam.name, "Sam Rivera");
        assert!(!sam.removed);
        assert!(
            mapped
                .saved_phones
                .iter()
                .any(|phone| phone == "447700900111")
        );
        assert!(
            !mapped
                .deleted_chats
                .iter()
                .any(|phone| phone == "447700900111")
        );

        let ada = phone_of(&mapped.contacts, "447700900222");
        assert_eq!(ada.name, "Ada Lovelace");
        assert!(!ada.removed);

        let unsaved = phone_of(&mapped.contacts, "447700900555");
        assert_eq!(unsaved.name, "");
        assert!(!unsaved.removed);

        assert_eq!(phone_of(&mapped.contacts, "447700900666").name, "Pat Lee");
        assert!(
            mapped
                .contacts
                .iter()
                .any(|contact| contact.phone == "447700900777" && !contact.removed)
        );
        assert!(
            !mapped
                .contacts
                .iter()
                .any(|contact| contact.name == "Ghost" || contact.phone == "999999999999999")
        );
        assert_eq!(mapped.explicit_removals, vec!["447700900444".to_owned()]);
        assert_eq!(mapped.deleted_chats, vec!["447700900888".to_owned()]);
        assert_eq!(mapped.groups.len(), 1);
        assert_eq!(mapped.groups[0].id, "120363111@g.us");
        assert_eq!(mapped.groups[0].subject, "School Gate");
        assert_eq!(mapped.deleted_group_ids, vec!["120363222@g.us".to_owned()]);
        assert!(
            !mapped
                .groups
                .iter()
                .any(|group| group.id.contains("broadcast") || group.id.contains("newsletter"))
        );
    }

    #[test]
    fn a_later_chat_delete_drops_that_chat_only_and_keeps_a_saved_person() {
        let mut book = LearnedBook::default();
        book.publish(map_sync(SyncBatch {
            contacts: &[row(
                "447700900111@s.whatsapp.net",
                None,
                Some("Sam Rivera"),
                Some(true),
                false,
            )],
            chats: &[HistoryChat {
                id: "447700900888@s.whatsapp.net",
                name: Some(""),
                display_name: None,
                pn_jid: None,
            }],
            ..SyncBatch::empty()
        }));
        let saved_chat = "447700900111@s.whatsapp.net";
        let deleted_saved = book.publish(map_sync(SyncBatch {
            deleted_chat_ids: &[saved_chat],
            ..SyncBatch::empty()
        }));
        assert!(
            deleted_saved.is_empty(),
            "deleting the chat must not drop the saved person, got {deleted_saved:?}"
        );
        assert_eq!(book.snapshot_contacts().len(), 2);

        let unsaved_chat = "447700900888@s.whatsapp.net";
        let deleted_chat = book.publish(map_sync(SyncBatch {
            deleted_chat_ids: &[unsaved_chat],
            ..SyncBatch::empty()
        }));
        match deleted_chat.as_slice() {
            [ChannelEvent::Contacts(contacts)] => {
                assert_eq!(contacts.len(), 1);
                assert_eq!(contacts[0].phone, "447700900888");
                assert!(contacts[0].removed);
            }
            other => panic!("unsaved chat delete must drop that chat only, got {other:?}"),
        }
        assert_eq!(book.snapshot_contacts().len(), 1);
        assert_eq!(book.snapshot_contacts()[0].phone, "447700900111");

        book.publish(map_sync(SyncBatch {
            contacts: &[row(
                "447700900111@s.whatsapp.net",
                None,
                Some(""),
                Some(false),
                true,
            )],
            ..SyncBatch::empty()
        }));
        assert_eq!(book.snapshot_contacts().len(), 0);
    }

    #[test]
    fn contacts_learned_while_a_pull_is_running_are_in_the_snapshot() {
        let mut book = LearnedBook::default();
        book.publish(map_sync(SyncBatch {
            contacts: &[row(
                "447700900111@s.whatsapp.net",
                None,
                Some("Sam Rivera"),
                Some(true),
                false,
            )],
            ..SyncBatch::empty()
        }));
        let pinned = "447700900777@s.whatsapp.net";
        book.publish(map_sync(SyncBatch {
            named_chat_ids: &[pinned],
            ..SyncBatch::empty()
        }));
        let snapshot = snapshot_keeping_contacts(book.snapshot_contacts(), Err(()));
        assert_eq!(snapshot.contacts.len(), 2);
        assert_eq!(snapshot.groups.len(), 0);
        assert!(
            snapshot
                .contacts
                .iter()
                .any(|contact| contact.phone == "447700900777" && !contact.removed)
        );
        let with_groups = snapshot_keeping_contacts(
            snapshot.contacts,
            Ok(vec![SyncedGroup {
                id: "120363111@g.us".into(),
                subject: "School Gate".into(),
            }]),
        );
        assert_eq!(with_groups.groups.len(), 1);
        assert_eq!(with_groups.contacts.len(), 2);
    }

    #[test]
    fn a_batch_larger_than_the_old_mailbox_is_one_contact_list() {
        let count = 1025;
        let ids: Vec<String> = (0..count)
            .map(|index| format!("4477{index:09}@s.whatsapp.net"))
            .collect();
        let names: Vec<String> = (0..count).map(|index| format!("Person {index}")).collect();
        let chats: Vec<HistoryChat<'_>> = ids
            .iter()
            .zip(names.iter())
            .map(|(id, name)| HistoryChat {
                id,
                name: Some(name),
                display_name: None,
                pn_jid: None,
            })
            .collect();
        let mapped = map_sync(SyncBatch {
            chats: &chats,
            ..SyncBatch::empty()
        });
        assert_eq!(mapped.contacts.len(), count);
        assert!(mapped.contacts.iter().all(|contact| !contact.removed));
        let mut book = LearnedBook::default();
        let events = book.publish(mapped);
        match events.as_slice() {
            [ChannelEvent::Contacts(contacts)] => assert_eq!(contacts.len(), count),
            other => panic!("the whole batch must be stored together, got {other:?}"),
        }
        assert_eq!(book.snapshot_contacts().len(), count);
    }

    #[test]
    fn history_sync_is_processed_not_acknowledged_and_discarded() {
        let src = include_str!("lib.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        assert!(
            !src.contains("skip_history_sync"),
            "ack-and-discard of INITIAL_BOOTSTRAP drops contacts and groups"
        );
    }
}
