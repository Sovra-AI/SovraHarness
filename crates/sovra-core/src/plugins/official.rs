//! Plugins Sovra itself publishes and supports. They are listed on the Plugins page
//! with plain words about what they do and install, switched on, with one click.
//!
//! Only these repositories can be installed through `PluginCommand::InstallOfficial`,
//! so the one-click path can never be pointed at an arbitrary address. They are still
//! instruction-only GitHub plugins: nothing in them runs unless the person approves it.

use super::{CODE_API_VERSION, GitHubRepository, PluginEntry, skill_command_prefix};
use crate::skills::SkillError;

/// One plugin made and kept working by the Sovra team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfficialPlugin {
    /// The public GitHub repository, as `owner/name`.
    pub repository: &'static str,
    /// A short everyday name shown instead of the repository name.
    pub title: &'static str,
    /// One sentence saying what it is for.
    pub summary: &'static str,
    /// What it lets Sovra do, one plain line each.
    pub does: &'static [&'static str],
    /// What the person needs before it is useful, in plain words.
    pub needs: &'static str,
    /// What it is not, when people could mix it up with something Sovra already has.
    pub good_to_know: &'static str,
    /// What the person can do once it is on, as buttons. The first one is the
    /// starting point, offered as **Try it in chat**.
    pub actions: &'static [OfficialAction],
}

/// One thing to do with an official plugin, started in chat with one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfficialAction {
    /// The button's words.
    pub label: &'static str,
    /// The skill it starts, by its `SKILL.md` name.
    pub skill: &'static str,
    /// The request it puts in the message box, ready to send.
    pub prompt: &'static str,
}

/// Every official plugin, in the order the Plugins page shows them.
pub const OFFICIAL_PLUGINS: &[OfficialPlugin] = &[OfficialPlugin {
    repository: "Sovra-AI/sovra-compute-pool",
    title: "Computer pool",
    summary: "Join the computers you already have into one big shared computer, and run large jobs such as training an AI model across all of them at once.",
    does: &[
        "Sets up a pool on this computer and shows you how to add your other computers",
        "Runs a job on whichever computers have room, including their graphics cards",
        "Shares files between the computers and brings the results back to you",
        "Shows what each computer in the pool is doing, and helps when something goes wrong",
    ],
    needs: "Works with Linux computers on your network or over the internet. Sovra asks before it installs or runs anything on a computer.",
    good_to_know: "This is separate from the Training page. Training teaches Sovra's own small models with your group; the pool runs programs you choose, such as your own training scripts, on your computers.",
    actions: &[
        OfficialAction {
            label: "Set up the pool",
            skill: "set-up-a-computer-pool",
            prompt: "Set up a computer pool on this computer so I can add my other computers to it.",
        },
        OfficialAction {
            label: "Add another computer",
            skill: "add-a-computer-to-the-pool",
            prompt: "Help me add another of my computers to my computer pool.",
        },
        OfficialAction {
            label: "Run a job",
            skill: "run-a-job-on-the-pool",
            prompt: "Help me run a program on my computer pool and bring the results back.",
        },
        OfficialAction {
            label: "Check on the pool",
            skill: "check-on-the-pool",
            prompt: "How is my computer pool doing? Which computers are in it and what are they busy with?",
        },
    ],
}];

impl OfficialPlugin {
    /// The repository this plugin is added from.
    pub fn github(&self) -> GitHubRepository {
        GitHubRepository::parse(self.repository).expect("official repositories are valid")
    }

    /// The plugin ID it is installed under, the same as adding the repository by hand.
    pub fn id(&self) -> String {
        self.github().plugin_id()
    }

    /// The starting point, offered as **Try it in chat**.
    pub fn starter(&self) -> &'static OfficialAction {
        &self.actions[0]
    }

    /// The chat command of an action's skill.
    pub fn command(&self, action: &OfficialAction) -> String {
        format!("{}{}", skill_command_prefix(&self.id()), action.skill)
    }
}

/// The official plugin installed under `id`, if any.
pub fn official_plugin(id: &str) -> Option<&'static OfficialPlugin> {
    OFFICIAL_PLUGINS.iter().find(|plugin| plugin.id() == id)
}

/// Whether a one-click install may save this package switched on. A package with
/// code never is: its code runs only after the person has seen what it can reach
/// and chosen **Allow and turn on** (`PluginCommand::AllowCode`), official or not.
pub fn turns_on_at_install(entry: &PluginEntry) -> bool {
    entry.api_version != CODE_API_VERSION
}

/// Looks up an official plugin for a one-click install, refusing any other ID.
pub fn require_official(id: &str) -> Result<&'static OfficialPlugin, SkillError> {
    official_plugin(id)
        .ok_or_else(|| SkillError("This plugin is not one of Sovra's official plugins.".into()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::skills::valid_name;

    #[test]
    fn every_official_plugin_is_a_valid_unique_repository() {
        let mut ids = BTreeSet::new();
        for plugin in OFFICIAL_PLUGINS {
            let repository = GitHubRepository::parse(plugin.repository)
                .unwrap_or_else(|error| panic!("{}: {error}", plugin.repository));
            assert_eq!(
                format!("{}/{}", repository.owner, repository.name),
                plugin.repository,
                "write the repository exactly as owner/name"
            );
            assert!(
                ids.insert(plugin.id()),
                "{} is listed twice",
                plugin.repository
            );
            assert_eq!(official_plugin(&plugin.id()), Some(plugin));
        }
    }

    #[test]
    fn official_plugin_copy_is_short_plain_and_complete() {
        for plugin in OFFICIAL_PLUGINS {
            let name = plugin.repository;
            assert!(
                !plugin.title.is_empty() && plugin.title.len() <= 40,
                "{name} title"
            );
            assert!(
                plugin.summary.ends_with('.') && plugin.summary.len() <= 200,
                "{name} summary"
            );
            assert!(
                (2..=5).contains(&plugin.does.len()),
                "{name} lists 2–5 things it does"
            );
            for line in plugin.does {
                assert!(
                    !line.is_empty() && line.len() <= 100 && !line.ends_with('.'),
                    "{line}"
                );
            }
            assert!(
                plugin.needs.ends_with('.') && plugin.needs.len() <= 200,
                "{name} needs"
            );
            assert!(plugin.good_to_know.ends_with('.') && plugin.good_to_know.len() <= 240);
            assert!(
                (1..=4).contains(&plugin.actions.len()),
                "{name} offers 1–4 actions"
            );
            let mut skills = BTreeSet::new();
            for action in plugin.actions {
                assert!(valid_name(action.skill), "{name}: {}", action.skill);
                assert!(skills.insert(action.skill), "{name}: one button per skill");
                assert!(!action.label.is_empty() && action.label.len() <= 24);
                assert!(!action.prompt.is_empty() && action.prompt.len() <= 200);
            }
            for jargon in ["agent", "persona", "repository", "CLI", "API", "cluster"] {
                let copy = [
                    plugin.title,
                    plugin.summary,
                    plugin.needs,
                    plugin.good_to_know,
                ]
                .into_iter()
                .chain(plugin.does.iter().copied())
                .chain(
                    plugin
                        .actions
                        .iter()
                        .flat_map(|action| [action.label, action.prompt]),
                );
                for text in copy {
                    assert!(
                        !text.contains(jargon),
                        "{name}: avoid “{jargon}” in “{text}”"
                    );
                }
            }
        }
    }

    #[test]
    fn one_click_never_switches_on_code_without_its_approval() {
        let entry = |api_version| PluginEntry {
            id: OFFICIAL_PLUGINS[0].id(),
            title: "Computer pool".into(),
            description: String::new(),
            version: "1.0.0".into(),
            api_version,
            package_url: String::new(),
            sha256: String::new(),
        };
        assert!(turns_on_at_install(&entry(1)));
        assert!(!turns_on_at_install(&entry(CODE_API_VERSION)));
    }

    #[test]
    fn only_official_ids_pass_the_one_click_check() {
        let pool = &OFFICIAL_PLUGINS[0];
        assert_eq!(pool.id(), "io.github.sovra-ai.sovra-compute-pool");
        let starter = pool.command(pool.starter());
        assert!(
            starter.starts_with(&skill_command_prefix(&pool.id()))
                && starter.ends_with("set-up-a-computer-pool")
        );
        assert_eq!(require_official(&pool.id()).ok(), Some(pool));
        for other in [
            "io.github.sovra-ai.sovraharness",
            "io.github.someone.sovra-compute-pool",
            "",
        ] {
            assert!(require_official(other).is_err(), "{other}");
        }
    }
}
