//! One person, many ways to reach them.
//!
//! Phone numbers and email addresses join records of the same person. A shared
//! first name never does. Telegram is only a chat that is already known.

use serde::{Deserialize, Serialize};

use crate::connections::{ChannelError, ChannelKind};

/// Shown when `WhatsApp` has linked and shared nobody yet.
pub const EMPTY_PEOPLE_NOTE: &str = "No one has synced yet. When WhatsApp shares people from your phone, they show up here. You can message them before they write to you.";

/// Shown when a contact pull fails. People already saved stay in the list.
pub const PEOPLE_PULL_FAILED_NOTE: &str =
    "WhatsApp could not share people just now. Anyone already saved stays in the list.";

/// A way to reach someone. Everyday words, not service jargon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reach {
    Phone,
    WhatsApp,
    Telegram,
    Email,
}

impl Reach {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Phone => "Phone",
            Self::WhatsApp => "WhatsApp",
            Self::Telegram => "Telegram",
            Self::Email => "Email",
        }
    }
}

/// One saved way to reach a person. `key` is the join key, not the words on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handle {
    pub reach: Reach,
    pub key: String,
    pub label: String,
}

/// A person in the list. The id stays stable when handles are added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub handles: Vec<Handle>,
}

impl Person {
    /// The name on the card. A missing name uses the first saved handle.
    pub fn title(&self) -> String {
        let name = clean_name(&self.name);
        if !name.is_empty() {
            return name;
        }
        self.handles
            .first()
            .map_or_else(|| "Saved person".to_owned(), |handle| handle.label.clone())
    }

    /// Channel words in a stable order, with no duplicates.
    pub fn channel_labels(&self) -> Vec<&'static str> {
        [Reach::Phone, Reach::WhatsApp, Reach::Telegram, Reach::Email]
            .into_iter()
            .filter(|reach| self.handles.iter().any(|handle| handle.reach == *reach))
            .map(Reach::label)
            .collect()
    }
}

/// Everyone Sovra can message, plus a short note when the list is empty or a pull failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PeopleBook {
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub note: Option<String>,
}

/// A contact the linked `WhatsApp` account synced. Groups and hidden ids never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedContact {
    pub name: String,
    pub phone: String,
    pub removed: bool,
}

/// What a typed name means. More than one match is never collapsed to the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatch {
    None,
    One(String),
    Many,
}

/// What the helper may see. The full list is never returned for an empty question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Ask {
        count: usize,
    },
    None,
    One {
        name: String,
        channels: Vec<&'static str>,
    },
    Many {
        names: Vec<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JoinClass {
    Phone,
    Email,
    Telegram,
}

impl PeopleBook {
    pub fn note_empty_sync(&mut self) {
        if self.people.is_empty() {
            self.note = Some(EMPTY_PEOPLE_NOTE.to_owned());
        }
    }

    pub fn note_pull_failed(&mut self) {
        self.note = Some(PEOPLE_PULL_FAILED_NOTE.to_owned());
    }

    /// Record one synced `WhatsApp` contact. A removal drops only that `WhatsApp` number.
    pub fn apply_synced(&mut self, contact: &SyncedContact) {
        if contact.removed {
            let _ = self.forget_whatsapp_number(&contact.phone);
            return;
        }
        let _ = self.observe_handle(Reach::WhatsApp, &contact.phone, &contact.name);
    }

    /// Add a handle from something the person typed or a channel already reported.
    ///
    /// The same phone or email joins onto the person who already has it.
    pub fn observe_handle(&mut self, reach: Reach, value: &str, name: &str) -> bool {
        let Some(key) = normalize(reach, value) else {
            return false;
        };
        let _ = self.insert(reach, key, name, None);
        true
    }

    /// Add a handle to one chosen person, joining anyone else who already has it.
    /// The returned id is the person who kept the handle.
    pub fn attach(&mut self, person_id: &str, reach: Reach, value: &str) -> Result<String, String> {
        if !self.people.iter().any(|person| person.id == person_id) {
            return Err("That person is not in the list.".into());
        }
        let key = normalize(reach, value).ok_or_else(|| hint(reach))?;
        Ok(self.insert(reach, key, "", Some(person_id)))
    }

    /// Join two records the person says are the same. The lower id is kept.
    pub fn link_people(&mut self, a: &str, b: &str) -> Option<String> {
        if a == b
            || !self.people.iter().any(|person| person.id == a)
            || !self.people.iter().any(|person| person.id == b)
        {
            return None;
        }
        let (keep, drop) = if person_number(a) <= person_number(b) {
            (a, b)
        } else {
            (b, a)
        };
        self.merge_into(keep, drop);
        Some(keep.to_owned())
    }

    /// Remove one channel. The person and every other channel stay.
    pub fn drop_reach(&mut self, person_id: &str, reach: Reach) -> bool {
        let Some(person) = self.people.iter_mut().find(|person| person.id == person_id) else {
            return false;
        };
        let before = person.handles.len();
        person.handles.retain(|handle| handle.reach != reach);
        before != person.handles.len()
    }

    pub fn resolve_name(&self, query: &str) -> NameMatch {
        match self.matching(query).as_slice() {
            [] => NameMatch::None,
            [index] => NameMatch::One(self.people[*index].id.clone()),
            _ => NameMatch::Many,
        }
    }

    pub fn lookup(&self, query: &str) -> Lookup {
        if query.trim().is_empty() {
            return Lookup::Ask {
                count: self.people.len(),
            };
        }
        let indexes = self.matching(query);
        match indexes.as_slice() {
            [] => Lookup::None,
            [index] => {
                let person = &self.people[*index];
                Lookup::One {
                    name: person.title(),
                    channels: person.channel_labels(),
                }
            }
            many => Lookup::Many {
                names: many
                    .iter()
                    .take(8)
                    .map(|index| self.people[*index].title())
                    .collect(),
            },
        }
    }

    /// The address a review can use. A phone number alone is not a `WhatsApp` handle.
    pub fn messaging_address(
        &self,
        person_id: &str,
        channel: ChannelKind,
    ) -> Result<(String, String), ChannelError> {
        let Some(person) = self.people.iter().find(|person| person.id == person_id) else {
            return Err(ChannelError::Failed(
                "That person is not in the list.".into(),
            ));
        };
        let label = person.title();
        let reach = match channel {
            ChannelKind::WhatsApp => Reach::WhatsApp,
            ChannelKind::Telegram => Reach::Telegram,
        };
        let handles: Vec<_> = person
            .handles
            .iter()
            .filter(|handle| handle.reach == reach)
            .collect();
        match (channel, handles.as_slice()) {
            (ChannelKind::WhatsApp, [handle]) => {
                Ok((format!("{}@s.whatsapp.net", handle.key), label))
            }
            (ChannelKind::WhatsApp, []) => Err(ChannelError::Failed(format!(
                "{label} is saved, but not on WhatsApp yet. Add a WhatsApp number, or use a phone number starting with +."
            ))),
            (ChannelKind::WhatsApp, _) => Err(ChannelError::Failed(format!(
                "{label} has more than one WhatsApp number. Use the number starting with +."
            ))),
            (ChannelKind::Telegram, [handle]) => Ok((handle.key.clone(), label)),
            (ChannelKind::Telegram, []) => Err(ChannelError::Failed(format!(
                "{label} has no Telegram chat yet. Telegram only knows chats that have already written to your bot, or a chat you add."
            ))),
            (ChannelKind::Telegram, _) => Err(ChannelError::Failed(format!(
                "{label} has more than one Telegram chat. Say which chat number to use."
            ))),
        }
    }

    fn insert(&mut self, reach: Reach, key: String, name: &str, prefer: Option<&str>) -> String {
        if let Some(person_id) = prefer
            && let Some(person) = self.people.iter_mut().find(|person| person.id == person_id)
            && !person
                .handles
                .iter()
                .any(|handle| handle.reach == reach && handle.key == key)
        {
            person.handles.push(handle(reach, key.clone()));
        }
        let owners = self.owner_ids(reach, &key);
        if owners.is_empty() {
            self.next_id = self.next_id.saturating_add(1);
            let id = format!("person-{}", self.next_id);
            self.people.push(Person {
                id: id.clone(),
                name: clean_name(name),
                handles: vec![handle(reach, key)],
            });
            self.note = None;
            return id;
        }
        let keep = self.collapse(&owners);
        if let Some(person) = self.people.iter_mut().find(|person| person.id == keep) {
            if !person
                .handles
                .iter()
                .any(|existing| existing.reach == reach && existing.key == key)
            {
                person.handles.push(handle(reach, key));
            }
            if person.name.trim().is_empty() {
                person.name = clean_name(name);
            }
        }
        self.note = None;
        keep
    }

    fn collapse(&mut self, ids: &[String]) -> String {
        let mut ids = ids.to_vec();
        ids.sort_by_key(|id| person_number(id));
        ids.dedup();
        let Some(keep) = ids.first().cloned() else {
            return String::new();
        };
        for drop_id in ids.iter().skip(1) {
            self.merge_into(&keep, drop_id);
        }
        keep
    }

    fn merge_into(&mut self, keep_id: &str, drop_id: &str) {
        let Some(dropped) = self
            .people
            .iter()
            .find(|person| person.id == drop_id)
            .cloned()
        else {
            return;
        };
        if let Some(keep) = self.people.iter_mut().find(|person| person.id == keep_id) {
            if keep.name.trim().is_empty() {
                keep.name.clone_from(&dropped.name);
            }
            for incoming in dropped.handles {
                if !keep
                    .handles
                    .iter()
                    .any(|handle| handle.reach == incoming.reach && handle.key == incoming.key)
                {
                    keep.handles.push(incoming);
                }
            }
        }
        self.people.retain(|person| person.id != drop_id);
    }

    fn forget_whatsapp_number(&mut self, phone: &str) -> bool {
        let Some(key) = phone_digits(phone) else {
            return false;
        };
        let mut changed = false;
        for person in &mut self.people {
            let before = person.handles.len();
            person
                .handles
                .retain(|handle| !(handle.reach == Reach::WhatsApp && handle.key == key));
            changed |= person.handles.len() != before;
        }
        changed
    }

    fn owner_ids(&self, reach: Reach, key: &str) -> Vec<String> {
        let class = join_class(reach);
        self.people
            .iter()
            .filter(|person| {
                person
                    .handles
                    .iter()
                    .any(|handle| join_class(handle.reach) == class && handle.key == key)
            })
            .map(|person| person.id.clone())
            .collect()
    }

    fn matching(&self, query: &str) -> Vec<usize> {
        let query = normalized(query);
        if query.is_empty() {
            return Vec::new();
        }
        let single = !query.contains(' ');
        self.people
            .iter()
            .enumerate()
            .filter(|(_, person)| {
                let title = normalized(&person.title());
                title == query
                    || (single
                        && title
                            .split_once(' ')
                            .is_some_and(|(first, _)| first == query))
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// Digits with a country code, 7–15 long, no leading zero.
/// A `@lid`, group, or other server is not a phone number.
pub fn phone_digits(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let user = if let Some((user, server)) = trimmed.split_once('@') {
        let server = server.split(':').next().unwrap_or(server);
        if server != "s.whatsapp.net" {
            return None;
        }
        user.split(':').next().unwrap_or(user)
    } else {
        trimmed
    };
    if user
        .chars()
        .any(|c| !(c.is_ascii_digit() || c == '+' || c == ' ' || c == '(' || c == ')' || c == '-'))
    {
        return None;
    }
    let digits: String = user.chars().filter(char::is_ascii_digit).collect();
    if (7..=15).contains(&digits.len()) && !digits.starts_with('0') {
        Some(digits)
    } else {
        None
    }
}

fn normalize(reach: Reach, value: &str) -> Option<String> {
    match reach {
        Reach::Phone | Reach::WhatsApp => phone_digits(value),
        Reach::Email => email_key(value),
        Reach::Telegram => telegram_key(value),
    }
}

fn email_key(input: &str) -> Option<String> {
    let email = input.trim().to_ascii_lowercase();
    let (local, domain) = email.split_once('@')?;
    if local.is_empty()
        || local.contains(' ')
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c))
    {
        return None;
    }
    let labels: Vec<_> = domain.split('.').collect();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return None;
    }
    Some(email)
}

fn telegram_key(input: &str) -> Option<String> {
    let id = input.trim();
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
        Some(id.to_owned())
    } else {
        None
    }
}

fn handle(reach: Reach, key: String) -> Handle {
    let label = match reach {
        Reach::Phone | Reach::WhatsApp => format!("+{key}"),
        Reach::Email => key.clone(),
        Reach::Telegram => format!("Chat {key}"),
    };
    Handle { reach, key, label }
}

fn join_class(reach: Reach) -> JoinClass {
    match reach {
        Reach::Phone | Reach::WhatsApp => JoinClass::Phone,
        Reach::Email => JoinClass::Email,
        Reach::Telegram => JoinClass::Telegram,
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalized(text: &str) -> String {
    clean_name(text).to_lowercase()
}

fn person_number(id: &str) -> u64 {
    id.strip_prefix("person-")
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

fn hint(reach: Reach) -> String {
    match reach {
        Reach::Email => "Enter an email address like sam@example.com.".into(),
        Reach::Telegram => "Enter the chat number Telegram already gave this chat.".into(),
        Reach::Phone | Reach::WhatsApp => {
            "Enter a phone number with the country code, like +44 7700 900123.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communications::resolve_recipient;
    use crate::connections::ConnectionsState;

    fn book_with_sam() -> PeopleBook {
        let mut book = PeopleBook::default();
        assert!(book.observe_handle(Reach::WhatsApp, "+44 7700 900111", "Sam Rivera"));
        book
    }

    #[test]
    fn the_same_phone_joins_whatsapp_and_a_phone_number() {
        let mut book = book_with_sam();
        assert!(book.observe_handle(Reach::Phone, "447700900111", ""));
        assert_eq!(book.people.len(), 1);
        assert_eq!(book.people[0].id, "person-1");
        assert_eq!(book.people[0].name, "Sam Rivera");
        assert!(
            book.people[0]
                .handles
                .iter()
                .any(|h| h.reach == Reach::WhatsApp)
        );
        assert!(
            book.people[0]
                .handles
                .iter()
                .any(|h| h.reach == Reach::Phone)
        );
        assert_eq!(book.people[0].handles.len(), 2);
    }

    #[test]
    fn the_same_email_joins_and_a_different_email_does_not() {
        let mut book = PeopleBook::default();
        assert!(book.observe_handle(Reach::Email, "Sam@Example.com", "Sam Rivera"));
        assert!(book.observe_handle(Reach::Email, "sam@example.com", "Someone else"));
        assert_eq!(book.people.len(), 1);
        assert_eq!(book.people[0].name, "Sam Rivera");
        assert!(book.observe_handle(Reach::Email, "ada@example.com", "Ada Lovelace"));
        assert_eq!(book.people.len(), 2);
    }

    #[test]
    fn two_people_who_share_only_a_first_name_stay_separate() {
        let mut book = book_with_sam();
        assert!(book.observe_handle(Reach::WhatsApp, "+447700900222", "Sam Okonkwo"));
        assert_eq!(book.people.len(), 2);
        assert!(matches!(book.resolve_name("Sam"), NameMatch::Many));
        assert!(matches!(book.resolve_name("Sam Rivera"), NameMatch::One(_)));
        assert!(matches!(
            book.resolve_name("Sam Okonkwo"),
            NameMatch::One(_)
        ));
        assert!(matches!(book.resolve_name("Rivera"), NameMatch::None));
    }

    #[test]
    fn the_same_full_name_on_two_numbers_does_not_join() {
        let mut book = PeopleBook::default();
        assert!(book.observe_handle(Reach::WhatsApp, "+447700900111", "Sam Rivera"));
        assert!(book.observe_handle(Reach::WhatsApp, "+447700900222", "Sam Rivera"));
        assert_eq!(book.people.len(), 2);
        assert!(matches!(book.resolve_name("Sam Rivera"), NameMatch::Many));
    }

    #[test]
    fn an_explicit_link_joins_people_who_do_not_share_a_number() {
        let mut book = book_with_sam();
        assert!(book.observe_handle(Reach::Email, "ada@example.com", "Ada Lovelace"));
        let first = book.people[0].id.clone();
        let second = book.people[1].id.clone();
        let kept = book.link_people(&first, &second).expect("joined");
        assert_eq!(kept, "person-1");
        assert_eq!(book.people.len(), 1);
        assert_eq!(book.people[0].channel_labels(), ["WhatsApp", "Email"]);
        assert!(book.observe_handle(Reach::Phone, "+15550000001", "New Person"));
        assert_eq!(book.people[1].id, "person-3");
    }

    #[test]
    fn attaching_a_known_handle_joins_and_dropping_one_channel_leaves_the_others() {
        let mut book = book_with_sam();
        assert!(book.observe_handle(Reach::Telegram, "4242", "Sam"));
        assert_eq!(book.people.len(), 2, "a shared first name is not a join");
        let first = book.people[0].id.clone();
        let kept = book
            .attach(&first, Reach::Telegram, "4242")
            .expect("chat attached");
        assert_eq!(book.people.len(), 1);
        assert_eq!(kept, "person-1");
        let email = book
            .attach("person-1", Reach::Email, "sam.rivera@example.com")
            .expect("email");
        assert_eq!(email, "person-1");
        assert!(book.drop_reach("person-1", Reach::WhatsApp));
        assert_eq!(book.people.len(), 1);
        assert_eq!(book.people[0].channel_labels(), ["Telegram", "Email"]);
        assert!(!book.drop_reach("person-1", Reach::Phone));
    }

    #[test]
    fn an_ambiguous_name_does_not_resolve_to_one_person() {
        let mut state = ConnectionsState::default();
        state
            .people
            .observe_handle(Reach::WhatsApp, "+447700900111", "Sam Rivera");
        let own = "15550000000@s.whatsapp.net";
        let (jid, label) =
            resolve_recipient(&state, ChannelKind::WhatsApp, "Sam Rivera", own).unwrap();
        assert_eq!(jid, "447700900111@s.whatsapp.net");
        assert_eq!(label, "Sam Rivera");
        assert_eq!(state.recent_messages.len(), 0);
        state
            .people
            .observe_handle(Reach::WhatsApp, "+447700900222", "Sam Okonkwo");
        let ambiguous = resolve_recipient(&state, ChannelKind::WhatsApp, "Sam", own).unwrap_err();
        assert!(
            ambiguous.to_string().contains("More than one"),
            "{ambiguous}"
        );
        assert!(resolve_recipient(&state, ChannelKind::WhatsApp, "Sam Rivera", own).is_ok());
        let mut phone_only = ConnectionsState::default();
        phone_only
            .people
            .observe_handle(Reach::Phone, "+447700900111", "Sam Rivera");
        assert!(resolve_recipient(&phone_only, ChannelKind::WhatsApp, "Sam Rivera", own).is_err());
        let mut named = ConnectionsState::default();
        named
            .people
            .observe_handle(Reach::WhatsApp, "+447700900333", "Ada Lovelace");
        named.people.drop_reach("person-1", Reach::WhatsApp);
        named
            .people
            .attach("person-1", Reach::Email, "ada@example.com")
            .unwrap();
        assert_eq!(named.people.people[0].channel_labels(), ["Email"]);
        assert!(resolve_recipient(&named, ChannelKind::WhatsApp, "Ada Lovelace", own).is_err());
    }

    #[test]
    fn a_removed_whatsapp_number_leaves_the_other_channels() {
        let mut book = book_with_sam();
        book.attach("person-1", Reach::Email, "sam@example.com")
            .unwrap();
        book.apply_synced(&SyncedContact {
            name: String::new(),
            phone: "+447700900111".into(),
            removed: true,
        });
        assert_eq!(book.people[0].channel_labels(), ["Email"]);
        book.note_pull_failed();
        assert!(book.note.as_deref().unwrap().contains("already saved"));
        assert_eq!(book.people.len(), 1);
    }

    #[test]
    fn an_empty_sync_names_nobody_and_a_blank_question_lists_nobody() {
        let mut book = PeopleBook::default();
        book.note_empty_sync();
        assert_eq!(book.people.len(), 0);
        assert!(book.note.as_deref().unwrap().contains("before they write"));
        book.observe_handle(Reach::WhatsApp, "+447700900111", "Sam Rivera");
        assert!(book.note.is_none());
        assert!(matches!(book.lookup(""), Lookup::Ask { count: 1 }));
        assert!(matches!(book.lookup("Sam"), Lookup::One { .. }));
        book.observe_handle(Reach::WhatsApp, "+447700900222", "Sam Okonkwo");
        assert!(matches!(book.lookup("Sam"), Lookup::Many { .. }));
    }

    #[test]
    fn older_connections_json_without_a_book_starts_empty() {
        let mut value = serde_json::to_value(ConnectionsState::default()).unwrap();
        value.as_object_mut().unwrap().remove("people");
        let state: ConnectionsState = serde_json::from_value(value).unwrap();
        assert_eq!(state.people.people.len(), 0);
    }
}
