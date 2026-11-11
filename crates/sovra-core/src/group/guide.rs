//! The Groups page explained in plain words: what a group is for, the steps to get
//! one going, the pages inside it, and what a new group still has to do. Pure data
//! and rules, so the wording and the "what's next" list are unit-tested here; the
//! UI only lays them out.

use serde::{Deserialize, Serialize};

use super::session::GroupsView;

/// One thing a group lets people do, for the "What is a group?" card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Benefit {
    pub icon: &'static str,
    pub title: &'static str,
    pub example: &'static str,
}

/// The first sentence anyone reads about groups.
pub const WHAT_IS_A_GROUP: &str = "A group joins your Sovra to the Sovra on the computers of people you trust, like your family or a small team at work. Then you can share things with all of them at once.";

/// Why it is safe, said once and plainly.
pub const PRIVACY_PROMISE: &str = "Private: nothing is shared until you press Share. What you share goes straight from your computer to theirs, locked so only your group can read it. It never passes through a Sovra server.";

pub const BENEFITS: [Benefit; 4] = [
    Benefit {
        icon: "📝",
        title: "Share notes",
        example: "Tell everyone's helpers something once, like “Grandad's birthday is 4 June”.",
    },
    Benefit {
        icon: "📄",
        title: "Share documents and helpers",
        example: "Send a shopping list, a letter or a helper you made to everyone at once.",
    },
    Benefit {
        icon: "🔎",
        title: "Ask each other",
        example: "Ask “Who is our plumber?” and get an answer from the family's computers.",
    },
    Benefit {
        icon: "💻",
        title: "Borrow an AI",
        example: "An old laptop with no AI can use the AI on another computer in the group.",
    },
];

/// How a group comes together, in order.
pub const HOW_IT_WORKS: [&str; 3] = [
    "One person starts the group on their computer.",
    "They invite everyone else: with a link sent in a message, or with six numbers if you're in the same room.",
    "Each person chooses what to share. Nothing is shared until you press Share.",
];

/// The pages inside a group, in the order the page lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupPage {
    People,
    Invite,
    Notes,
    Helpers,
    Ask,
    BorrowAi,
    Departments,
    History,
}

impl GroupPage {
    pub const ALL: [Self; 8] = [
        Self::Invite,
        Self::Notes,
        Self::People,
        Self::Helpers,
        Self::Ask,
        Self::BorrowAi,
        Self::Departments,
        Self::History,
    ];

    /// Stable id for controls and tests.
    pub const fn id(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Invite => "invite",
            Self::Notes => "notes",
            Self::Helpers => "helpers",
            Self::Ask => "ask",
            Self::BorrowAi => "borrow-ai",
            Self::Departments => "departments",
            Self::History => "history",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::People => "👥",
            Self::Invite => "✉️",
            Self::Notes => "📝",
            Self::Helpers => "🤝",
            Self::Ask => "🔎",
            Self::BorrowAi => "💻",
            Self::Departments => "🏢",
            Self::History => "📜",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::People => "People",
            Self::Invite => "Invite someone",
            Self::Notes => "Shared notes",
            Self::Helpers => "Shared helpers",
            Self::Ask => "Ask the group",
            Self::BorrowAi => "Borrow an AI",
            Self::Departments => "Departments",
            Self::History => "History",
        }
    }

    /// One sentence: what this page is for.
    pub const fn what(self) -> &'static str {
        match self {
            Self::People => "Everyone in your group, and the computers they use.",
            Self::Invite => {
                "Bring a new person into the group, or add another of your own computers."
            }
            Self::Notes => "Tell your group something once, and everyone's helpers will know it.",
            Self::Helpers => {
                "Helpers, skills and schedules that people in your group made and shared."
            }
            Self::Ask => "Ask a question and get answers from what the other computers remember.",
            Self::BorrowAi => "A computer with no AI set up can use another group computer's AI.",
            Self::Departments => {
                "Smaller circles inside your group, like “Kids” or “Legal”. Notes shared with one only reach the people in it."
            }
            Self::History => {
                "A record of everything this computer shared, so you can always check."
            }
        }
    }

    /// Numbered steps: how to use this page.
    pub const fn how(self) -> &'static [&'static str] {
        match self {
            Self::People => &[
                "Each person can have more than one computer, like a laptop and a desktop.",
                "An admin can invite and remove people. Make a second admin so the group is never stuck if a computer is lost.",
                "To leave the group, press Leave this group at the bottom.",
            ],
            Self::Invite => &[
                "Type their name and press Create invite link. The link is copied for you.",
                "Paste it into an email or a message and send it to them.",
                "Keep Sovra open on this computer until they've joined. The link works once, for 10 minutes.",
                "Sitting next to them? Press We're in the same room: show six numbers, and read them out.",
            ],
            Self::Notes => &[
                "Type something useful, like “The dentist is Dr Patel, 01234 567890”.",
                "Choose who should see it: everyone, or just one person.",
                "Press Share. Anyone can then ask their helper, and it will answer with your note and say it came from you.",
            ],
            Self::Helpers => &[
                "Read what a helper or schedule says first.",
                "Press Use this helper, Use this skill or Add to my schedules to get your own copy.",
                "To share one of yours, open the Team or Schedules page and press Share with my group.",
            ],
            Self::Ask => &[
                "Type a question, like “Who is our plumber?”, and press Ask my group.",
                "Computers that allow it look in their own memory and send back short answers, each with a name on it.",
                "Your own memory is only searched if you turn on Answer questions from my group.",
            ],
            Self::BorrowAi => &[
                "On the computer that has an AI, turn on Lend this computer's AI.",
                "On the computer without one, turn on Borrow an AI when this computer has none.",
                "Then just chat as usual. Only the words of the chat are sent; nothing else on the helping computer is used.",
            ],
            Self::Departments => &[
                "An admin types a name and taps the people who belong.",
                "Press Save department.",
                "When sharing a note, choose Just and the department's name to keep it inside that circle.",
            ],
            Self::History => &[
                "Every share and every answer this computer gave is listed here, newest first.",
                "Each record is sealed, so if anything were ever changed you would see a warning.",
                "You don't need to do anything here. It's for peace of mind.",
            ],
        }
    }
}

/// One line of the "Getting started" list a new group shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstStep {
    pub title: &'static str,
    pub done: bool,
    /// The page that does it; `None` once done or when nothing needs opening.
    pub page: Option<GroupPage>,
}

impl GroupsView {
    /// The page shown first: an admin alone in a new group invites someone,
    /// everyone else lands on the shared notes.
    pub fn starting_page(&self) -> GroupPage {
        let alone = self.group.as_ref().is_some_and(|g| g.people.len() < 2);
        if alone && self.can_manage() {
            GroupPage::Invite
        } else {
            GroupPage::Notes
        }
    }

    /// What a group still has to do, in order. Empty outside a group, and empty
    /// once everything is done so the list goes away.
    pub fn first_steps(&self) -> Vec<FirstStep> {
        let Some(group) = &self.group else {
            return Vec::new();
        };
        let manage = self.can_manage();
        let invited = group.people.len() > 1;
        let shared =
            !self.knowledge.is_empty() || !self.artifacts.is_empty() || !self.templates.is_empty();
        let mut steps = vec![FirstStep {
            title: "Start or join a group",
            done: true,
            page: None,
        }];
        if manage {
            steps.push(step("Invite someone", invited, GroupPage::Invite));
        }
        steps.push(step("Share your first note", shared, GroupPage::Notes));
        if manage && invited {
            steps.push(step(
                "Make a second admin",
                !group.needs_second_admin(),
                GroupPage::People,
            ));
        }
        if steps.iter().all(|step| step.done) {
            return Vec::new();
        }
        steps
    }
}

fn step(title: &'static str, done: bool, page: GroupPage) -> FirstStep {
    FirstStep {
        title,
        done,
        page: (!done).then_some(page),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_says_what_it_is_and_how_to_use_it() {
        let mut ids = std::collections::BTreeSet::new();
        for page in GroupPage::ALL {
            assert!(ids.insert(page.id()), "duplicate id {}", page.id());
            assert_ne!(page.title(), "");
            assert!(page.what().ends_with('.'), "{page:?}");
            assert!(!page.how().is_empty(), "{page:?} has no steps");
            for line in page.how() {
                assert!(!line.contains("lane"), "jargon on {page:?}: {line}");
                assert!(!line.contains("federat"), "jargon on {page:?}: {line}");
            }
        }
    }

    #[test]
    fn outside_a_group_there_is_nothing_left_to_do_and_notes_come_first() {
        let view = GroupsView::default();
        assert_eq!(view.first_steps(), Vec::new());
        assert_eq!(view.starting_page(), GroupPage::Notes);
    }
}
