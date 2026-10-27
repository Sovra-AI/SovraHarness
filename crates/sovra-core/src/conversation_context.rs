//! Bounded text-only working context. Saved UI actions and reasoning are never replayed.

use crate::{ChatMessage, MessageRole, teams::TranscriptLine};

pub const MAX_CONTEXT_MESSAGES: usize = 24;
pub const MAX_CONTEXT_TEXT_BYTES: usize = 24 * 1024;
const MAX_MESSAGE_BYTES: usize = 8 * 1024;
const SHORTENED: &str = "\n[Earlier message shortened]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    pub role: MessageRole,
    pub text: String,
    pub speaker: Option<String>,
}

/// Constructed from saved visible messages, never tools, widgets, attachments or thinking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationContext {
    messages: Vec<ContextMessage>,
    shortened: bool,
}
impl ConversationContext {
    /// The latest user message is the new request and is sent separately.
    pub fn before_current_turn(messages: &[ChatMessage]) -> Self {
        messages
            .iter()
            .rposition(|message| message.role == MessageRole::User)
            .map_or_else(Self::default, |end| Self::from_messages(&messages[..end]))
    }
    pub fn from_messages(messages: &[ChatMessage]) -> Self {
        Self::bounded(messages.iter().rev().map(|message| {
            (
                message.role,
                message.text.as_str(),
                message
                    .speaker
                    .as_ref()
                    .map(|speaker| speaker.name.as_str()),
            )
        }))
    }
    pub fn from_team(history: &[TranscriptLine]) -> Self {
        Self::bounded(history.iter().rev().map(|line| {
            (
                if line.speaker == "User" {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                line.text.as_str(),
                Some(line.speaker.as_str()),
            )
        }))
    }
    fn bounded<'a>(
        messages: impl Iterator<Item = (MessageRole, &'a str, Option<&'a str>)>,
    ) -> Self {
        let mut result = Self::default();
        let mut bytes = 0;
        for (role, text, speaker) in messages {
            if text.trim().is_empty() {
                continue;
            }
            if result.messages.len() == MAX_CONTEXT_MESSAGES || bytes >= MAX_CONTEXT_TEXT_BYTES {
                result.shortened = true;
                break;
            }
            let budget = (MAX_CONTEXT_TEXT_BYTES - bytes).min(MAX_MESSAGE_BYTES);
            if budget <= SHORTENED.len() {
                result.shortened = true;
                break;
            }
            let text = if text.len() > budget {
                let mut end = budget - SHORTENED.len();
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                result.shortened = true;
                format!("{}{SHORTENED}", &text[..end])
            } else {
                text.to_owned()
            };
            let speaker = speaker.map(|speaker| {
                let mut end = speaker.len().min(128);
                while !speaker.is_char_boundary(end) {
                    end -= 1;
                }
                speaker[..end].to_owned()
            });
            bytes += text.len();
            result.messages.push(ContextMessage {
                role,
                text,
                speaker,
            });
        }
        result.messages.reverse();
        result
    }
    pub fn messages(&self) -> &[ContextMessage] {
        &self.messages
    }
    pub fn shortened(&self) -> bool {
        self.shortened
    }
    pub fn team_history(&self) -> Vec<TranscriptLine> {
        self.messages
            .iter()
            .map(|message| TranscriptLine {
                speaker: match message.role {
                    MessageRole::User => "User".into(),
                    MessageRole::Assistant => {
                        message.speaker.clone().unwrap_or_else(|| "Sovra".into())
                    }
                },
                text: message.text.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_context_preserves_roles_without_replaying_private_or_interactive_state() {
        let mut assistant = ChatMessage::assistant(
            "a",
            "Done",
            "",
            vec![crate::default_personal_ai_build_widget()],
        );
        assistant.thinking = "private scratch work".into();
        let messages = vec![
            ChatMessage::user("u", "Use Rust", ""),
            assistant,
            ChatMessage::user("new", "Continue", ""),
        ];
        let context = ConversationContext::before_current_turn(&messages);
        assert_eq!(context.messages().len(), 2);
        assert_eq!(context.messages()[0].role, MessageRole::User);
        assert_eq!(context.messages()[1].role, MessageRole::Assistant);
        assert_eq!(context.messages()[1].text, "Done");
        assert!(!context.shortened());
        assert_eq!(context.team_history()[1].speaker, "Sovra");
        assert_eq!(
            ConversationContext::before_current_turn(&messages[1..2]),
            ConversationContext::default()
        );
    }
    #[test]
    fn large_unicode_history_keeps_a_bounded_recent_excerpt_with_explicit_omissions() {
        let messages = (0..100)
            .map(|i| ChatMessage::user(format!("u{i}"), format!("{i}:{}", "茶".repeat(9000)), ""))
            .collect::<Vec<_>>();
        let context = ConversationContext::from_messages(&messages);
        assert!(context.shortened());
        assert!(context.messages().len() <= MAX_CONTEXT_MESSAGES);
        assert!(
            context
                .messages()
                .iter()
                .map(|message| message.text.len())
                .sum::<usize>()
                <= MAX_CONTEXT_TEXT_BYTES
        );
        assert!(context.messages().last().unwrap().text.starts_with("99:"));
        assert!(context.messages().last().unwrap().text.ends_with(SHORTENED));
        let history = (0..50)
            .map(|i| TranscriptLine {
                speaker: "Helper".into(),
                text: format!("reply {i}"),
            })
            .collect::<Vec<_>>();
        let context = ConversationContext::from_team(&history);
        assert_eq!(context.messages().len(), MAX_CONTEXT_MESSAGES);
        assert_eq!(context.team_history().last().unwrap().text, "reply 49");
        assert!(context.shortened());
    }
}
