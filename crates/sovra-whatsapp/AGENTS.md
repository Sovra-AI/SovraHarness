# sovra-whatsapp

The only crate that names `whatsapp-rust` and `qrcode`. It implements the
`sovra_core::connections::MessagingChannel` port: open the WhatsApp Web link, turn the
client's events into `ChannelEvent`s, send text, log out. Credentials live in one SQLite
file under the app data directory (`whatsapp/session.sqlite`), never in the repository.

- Decide nothing here: which messages Sovra answers is `sovra_core::connections` policy.
- Plain words in every message that can reach the UI.
- Never log message bodies or phone numbers at info level.
