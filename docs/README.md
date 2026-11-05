# Docs

Map of the top-level guides for SovraHarness. Start at the
[root README](../README.md) for how to run the app and the architecture graphs.

| Guide | Purpose |
| ----- | ------- |
| [Architecture](architecture.md) | Why the crates, Command/Event, and adapters are shaped this way |
| [Native development](development.md) | One-shot `cargo xtask dev`, cache reclaim, sign-in status |
| [Development MCP](development-mcp.md) | Inspect and operate the running native window |
| [Memory capture and recall](memory-capture-and-recall.md) | Current contract for saving and recalling a chat turn |
| [Agent memory log](agent-memory.md) | Dated audits and measured limits; not the day-to-day contract |
| [Models](models.md) | Connect an AI service and pick a chat model |
| [Files in chat](files.md) | Paperclip attachments and the document reader helper |
| [File understanding](file-ontology.md) | What a file is, and live folder reading |
| [Skills and file knowledge](skills-and-file-knowledge.md) | Skills page, folder index, dated implementation log |
| [MCP servers](connected-services.md) | Extra tools from a URL, a local program, or pasted setup |
| [Messaging](communications.md) | WhatsApp, Telegram, and sending from desktop chat |
| [Google](google.md) | One-button Google connect and per-service switches |
| [Terminal](terminal.md) | Approved commands on this computer |
| [Teams and artifacts](teams-and-artifacts.md) | Helpers in chat and saveable work |
| [Styling](styling.md) | Native design system and responsive layout |

Investor and company slide decks live under [`decks/`](decks/README.md). Dated
evaluation dumps live under `evaluations/`. Neither is a product contract.
