# Docs

Map of the top-level guides for SovraHarness. Start at the
[root README](../README.md) for how to run the app and the architecture graphs.

| Guide | Purpose |
| ----- | ------- |
| [Getting started](getting-started.md) | First launch, first chat and finding features |
| [Configuration and local data](configuration.md) | Startup variables, data paths and troubleshooting |
| [Schedules](schedules.md) | Repeating prompts and confirmed timed messages |
| [Groups](groups.md) | How people and computers share a roster and explicit details |
| [Release guide](releasing.md) | Packaging, signing and release verification |
| [Roadmap](roadmap/README.md) | Implemented foundations and planned stages |
| [Sovereign swarm](swarm.md) | Company audit ledger, departments, sign-off and private shared training (rules built, not yet in the app) |
| [Architecture](architecture.md) | Why the crates, Command/Event, and adapters are shaped this way |
| [Native development](development.md) | One-shot `cargo xtask dev`, cache reclaim, sign-in status |
| [Development MCP](development-mcp.md) | Inspect and operate the running native window |
| [Phone remote control](remote-control.md) | Desktop pairing/control and the future mobile app contract |
| [Reviewing knowledge](knowledge.md) | Check sources, explore connections and stop recall |
| [Memory capture and recall](memory-capture-and-recall.md) | Current contract for saving and recalling a chat turn |
| [Agent memory log](agent-memory.md) | Dated audits and measured limits; not the day-to-day contract |
| [Knowledge benchmark](knowledge-benchmark.md) | Current tool assessment, reproducible retrieval diagnostic and verification gaps |
| [Models](models.md) | Connect an AI service and pick a chat model |
| [Files in chat](files.md) | Paperclip attachments and the document reader helper |
| [File understanding](file-ontology.md) | What a file is, and live folder reading |
| [Skills and file knowledge](skills-and-file-knowledge.md) | Skills page, folder index, dated implementation log |
| [MCP servers](connected-services.md) | Extra tools from a URL, a local program, or pasted setup |
| [Messaging](communications.md) | WhatsApp, Telegram, and sending from desktop chat |
| [Google](google.md) | One-button Google connect and per-service switches |
| [GitHub](github.md) | One-button GitHub connect (or a pasted key) and the GitHub helper's tools |
| [Terminal](terminal.md) | Approved commands on this computer |
| [The agent loop](agent-loop.md) | How helpers reason and act, loop guard rails, the live work log |
| [Teams and artifacts](teams-and-artifacts.md) | Helpers in chat and saveable work |
| [Styling](styling.md) | Native design system and responsive layout |

The enterprise case — sovereignty layer by layer with code evidence, value
drivers, how value is measured, and the vision from one computer to a
sovereign organisation — lives in
[`value-realisation/`](value-realisation/README.md).

Slide decks live under [`decks/`](decks/README.md) and open as one site from
[`decks/index.html`](decks/index.html):
[investors](decks/investor/index.html),
[company computers](decks/company-computers/index.html),
[a family or small business](decks/group/index.html),
[value you can count](decks/value/index.html),
[AI-native and 100% sovereign, for the UAE](decks/uae/index.html),
[the concise investor pitch](decks/pitch/index.html).
Dated evaluation dumps live under `evaluations/`. Neither is a product contract.

## Current guides and historical material

The guides above describe current behavior. The [roadmap](roadmap/README.md)
describes remaining work; implemented group foundations are covered in
[Groups](groups.md). Dated sections in the agent-memory and file-knowledge logs
record observations at that time and must not be read as current guarantees.

The [guided setup design](superpowers/specs/2026-10-24-guided-setup-and-system-tutorial-design.md)
and [implementation plan](superpowers/plans/2026-10-24-guided-setup-and-system-tutorial.md)
are historical design records. [Models](models.md) is the current setup guide.

For contributors: [workflow](../CONTRIBUTING.md), [security](../SECURITY.md),
[agent rules](../AGENTS.md), and [changelog](../CHANGELOG.md).

`cargo xtask check-docs` checks relative file links in the root README and
**top-level** `docs/*.md`, required Mermaid fences/names, and pitch slide 4 copy.
It does not render diagrams, validate anchors, check external URLs, or recursively
check the roadmap, decks and other nested Markdown. Review those separately.
