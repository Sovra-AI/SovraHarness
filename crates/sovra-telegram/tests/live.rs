//! Opt-in check against Telegram's real Bot API: a well-formed but made-up token is
//! refused by `getMe`, and the adapter turns that into the plain-words error the Settings
//! page shows. A real link needs a token from `@BotFather`, so that part stays manual.
//!
//! `cargo test -p sovra-telegram --test live -- --ignored --nocapture`

use std::sync::{Arc, Mutex};

use sovra_core::connections::{ChannelError, ChannelStartOptions, MessagingChannel};
use sovra_telegram::{TelegramChannel, TokenStore};

#[derive(Default)]
struct MemoryTokens(Mutex<Option<String>>);

impl TokenStore for MemoryTokens {
    fn load(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn save(&self, token: &str) -> Result<(), String> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(token.to_owned());
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to Telegram's servers over the internet"]
async fn telegram_refuses_a_made_up_token_in_plain_words() {
    let tokens = Arc::new(MemoryTokens::default());
    let channel = TelegramChannel::new(tokens.clone()).unwrap();
    let error = channel
        .start(
            ChannelStartOptions {
                phone_number: None,
                token: Some("123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw5".into()),
            },
            Arc::new(|_| {}),
        )
        .await
        .err()
        .expect("a made-up token is refused");
    eprintln!("{error}");
    assert!(
        matches!(&error, ChannelError::Failed(message) if message.contains("does not accept this token")),
        "{error}"
    );
    assert!(tokens.load().is_none(), "a refused token is never saved");
}
