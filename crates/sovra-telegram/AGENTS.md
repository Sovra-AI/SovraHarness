# sovra-telegram

Implements `sovra_core::connections::MessagingChannel` over Telegram's official Bot API
(HTTPS, long polling). No MTProto, no third-party client, no sidecar.

- The bot token is a secret: it goes through the `TokenStore` the composition root
  supplies (OS keychain in release builds), never into logs or the saved UI state.
- The runtime drives polling through `ChannelSession::poll`; this crate never spawns tasks
  or names tokio.
- Who may talk to the bot (owner only, everyone) is `sovra_core::connections` policy.
- Plain words in every message that can reach the UI or the phone.
