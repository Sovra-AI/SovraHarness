<p align="center">
  <img src="crates/sovra-ui/assets/branding/sovra-logo.svg" alt="Sovra logo" width="160">
</p>

<h1 align="center">SovraHarness</h1>

<p align="center">
  Your AI workspace, and a place to train your own small AI.<br>
  Privately, across your group's computers, in one app. No cloud. No Python.<br>
  Built in Rust with GPUI for macOS, Windows, and Linux.
</p>

<p align="center">
  <a href="docs/getting-started.md">Getting started</a>
  ·
  <a href="docs/training.md">Train your own AI</a>
  ·
  <a href="docs/README.md">Documentation</a>
  ·
  <a href="https://github.com/Sovra-AI/SovraHarness/releases">Releases</a>
  ·
  <a href="CONTRIBUTING.md">Contributing</a>
  ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

Connect a cloud AI service or a local model server such as Ollama, then use
helpers to work with your knowledge, create documents and websites, and
schedule tasks. Sovra also trains small AI models from your own text: on your
computer, with the computers in your group helping, and with the finished
model delivered to everyone in the group.

> **License:** Source on `main` is available under Sovra's Sustainable Use License.
> This is fair-code: source-available, and not an OSI open-source license.
> Enterprise files have separate terms. See [License](#license).

## Train your own small AI, privately, with your group

Pick some text, press **Start training**, and watch a model learn to write
like it. The training engine is inside the Sovra executable, so there is
nothing else to install: no cloud account, no Python, no separate trainer and
no model download.

```mermaid
sequenceDiagram
  participant Owner as Your computer
  participant Group as Your group's computers
  Owner->>Owner: Start training with Train with my group on
  Group->>Owner: Computers that chose to help ask for work
  Owner-->>Group: Short pieces of the chosen text and the current model
  Group->>Owner: Updates made of numbers, checked before they are kept
  Owner->>Owner: Finish and save the model
  Group->>Owner: Every group computer fetches the finished model
```

- **Simple to start.** Open **Training** and press
  **Start training**. A sample text, a model size suited to this computer and
  a balanced preset are already chosen.
- **Your group's computers help.** Leave **Train with my group** on. Computers
  in your group that turned on **Help the group train** join in by themselves,
  and a slow or missing one never holds the run up.
- **Everyone gets the model.** When the run finishes, every computer in the
  group receives the same model file, checked on arrival, including computers
  that were switched off at the time.
- **Watch it learn.** A live chart, plain-word numbers, time left, and a sample
  of what the model writes right now.
- **Choose a starting point.** Four built-in sizes, from a table that learns
  in seconds to a small transformer with 2.8 million values. Train any model
  in **Your models** further.
- **Simple presets, full control.** **Quick try**, **Balanced** and
  **Thorough**, with every setting available under **Advanced**.
- **Uses your graphics card when it helps.** Sovra checks the card itself and
  trains on it when it is clearly faster than the processor, with nothing to
  install or switch on. If the card fails mid-run, the run carries on with
  the processor.
- **See what this computer has.** Processor, cores, memory, the engine in use
  and which model sizes it is comfortable with.
- **Private by default.** Only the text you choose is used, and it leaves this
  computer only when you start a run with your group. Chats, memories and
  files are never added.

What to expect: these are small models that continue text in the style
you taught them. They do not replace the AI that answers chat, pretrained
models cannot be imported yet, and one graphics card at most is used.
Computers that help can read the pieces of text they work on, so share text
you are willing for your group to read. The [Training guide](docs/training.md)
has the first run, group steps, every setting, privacy details and limits.

## What you can do

- **Train your own small AI:** teach a model from your own text, with your
  group's computers helping, live progress, and the finished model delivered
  to everyone in the group. One app, no cloud, no Python.
- **Chat with helpers:** choose a model, discover capabilities on the Skills
  page, and follow the live work log as helpers work together.
- **Work with your knowledge:** attach files, index local folders, recall saved
  details, and explore their sources and connections.
- **Save your work:** organize versioned documents, code, and other artifacts
  into folders and favorites. Preview saved websites in your browser.
- **Connect services:** use Google, GitHub, WhatsApp, Telegram, and MCP tools.
  Connection setup and permissions depend on the service.
- **Schedule tasks:** run repeating prompts, confirm timed messages, or send a
  scheduled search note to a WhatsApp group.
- **Share with a group:** share selected knowledge, artifacts, helpers, and
  schedules across computers, with a signed history of sharing activity.

The interface includes a guided tour, a language picker, Arabic right-to-left
layout, and adjustable text size. See the [getting started guide](docs/getting-started.md)
for a tour of the features.

## Quick start

### Install a release

Open [GitHub Releases](https://github.com/Sovra-AI/SovraHarness/releases), choose
an asset for your operating system and architecture, and follow the release's
installation notes. Signing depends on the credentials available for that build;
check its release notes. In-app update checks are not implemented yet.

After launching:

1. Create or open a local account.
2. Follow the guided tour or open **Models** to connect a service and choose a
   chat model. Cloud services may require a key and charge for usage. For Ollama,
   install Ollama once; Sovra starts its local server when you use it.
   Sixteen other local-server presets, including LM Studio, llama.cpp, LocalAI,
   Jan and GPT4All, share compatible chat support. Start those servers in their
   own programs. See [local model setup](docs/local-models.md) for the full list,
   official references, startup behavior and troubleshooting.
3. Send a message in **Chat**, or open **Skills** to try an example.

To try training, open **Training** and press **Start training**. It needs no
chat provider or API key. To train with other computers, follow the
[group training steps](docs/training.md#train-with-your-group).

For connection problems, see [Models](docs/models.md) and
[Configuration and local data](docs/configuration.md).

### Build from source

Install Git and Rust through rustup. The repository pins Rust **1.99.1** in
[rust-toolchain.toml](rust-toolchain.toml). You also need your platform's build tools:

| Platform | Prerequisites |
| --- | --- |
| macOS | Xcode command-line tools |
| Windows | Visual Studio C++ build tools, Windows SDK, and the MSVC Rust target |
| Linux | System packages installed by `./scripts/install-linux-deps.sh` |

```sh
git clone https://github.com/Sovra-AI/SovraHarness.git
cd SovraHarness
# Linux only: ./scripts/install-linux-deps.sh
cargo xtask setup
cargo xtask dev
```

`cargo xtask dev` builds and launches once with `SOVRA_DEV=1`. Close the running
session and run it again after edits; there is no file watcher or hot reload.
Development mode enables local authenticated app control and stores development
keys in separate plaintext files outside the checkout (owner-only on Unix).
Ordinary launches and release builds start development control off; release keys
use the OS keychain. See [Native development](docs/development.md).

The native app does not require Node.js or Python. Building generated frontend
projects requires Node.js/npm and approved terminal access. Packaging has
additional prerequisites in the [release guide](docs/releasing.md).

## Status and data boundaries

Sovra is under active development. Read the [changelog](CHANGELOG.md) for changes
and the [documentation](docs/README.md) for current behavior.

- Local storage does not make every operation offline. Cloud models and connected
  tools receive the content needed for their requests; group networking can use
  discovery and encrypted relays. Review the [network inventory](docs/network.md).
- Local accounts do not provide separate encrypted data stores. Terminal commands
  and local MCP programs run with your OS permissions, without a sandbox.
- **My AIs** offers routing previews; these do not change the model answering chat
  or verify connection availability. Preview settings last for the session.
- Website hosting supports static output; server backends and server-side
  rendering are not hosted. Hosting stops when Sovra closes.
- Phone control pairs with the [SovraiOS](https://github.com/Sovra-AI/SovraiOS) and
  [SovraAndroid](https://github.com/Sovra-AI/SovraAndroid) apps, built from source;
  no store release is published yet. Groups support borrowing another computer's AI for chat; embedding and indexing work are not handed off.
- Training teaches small built-in models from text you choose; it does not
  import or fine-tune pretrained models. A run started with your group sends
  pieces of that text to group computers that help. See
  [Training: privacy and consent](docs/training.md#privacy-and-consent).

See [Security](SECURITY.md) for credential storage and execution boundaries.

## Development

| Command | Purpose |
| --- | --- |
| `cargo xtask dev` | One-shot development build and launch |
| `cargo xtask run` | Debug launch without `SOVRA_DEV` |
| `cargo xtask ci` | All required formatting, lint, architecture, docs, dependency, and test gates |
| `cargo xtask fix` | Apply formatting and Clippy fixes |
| `cargo xtask check-docs` | Check documentation links and required diagrams |
| `cargo xtask dist` | Build and verify a native installer |

See [CONTRIBUTING.md](CONTRIBUTING.md) for the pull request workflow and
[AGENTS.md](AGENTS.md) for engineering rules. To test the running app through an
MCP client, follow [Development MCP](docs/development-mcp.md).

### Architecture

Domain decisions live in `sovra-core`, IO lives in adapters, and views live in
`sovra-ui`. The UI sends commands and receives events through
`sovra_runtime::Client`.

```mermaid
flowchart TB
  app["sovra-app: composition root"] --> ui["sovra-ui: native views"]
  app --> runtime["sovra-runtime: async IO"]
  ui -->|"Command, e.g. RunPrompt"| runtime
  runtime -->|"Event"| ui
  ui --> core["sovra-core: types, reducers, ports"]
  runtime --> core
  runtime --> adapters["Adapters: models, files, storage, services"]
  adapters --> core
```

The enforced dependency rules live in [xtask/src/deps.rs](xtask/src/deps.rs).
Read [Architecture](docs/architecture.md) and
[Memory capture and recall](docs/memory-capture-and-recall.md) for the full design.

## Documentation

| Guide | Covers |
| --- | --- |
| [Training](docs/training.md) | Train your own small AI: first run, group training and delivery, live numbers, settings, privacy, and limits |
| [Getting started](docs/getting-started.md) | First launch, first chat, and finding features |
| [Configuration](docs/configuration.md) | Startup variables, data paths, and troubleshooting |
| [Models](docs/models.md) | Providers, keys, model selection, and routing previews |
| [Files](docs/files.md) and [Knowledge](docs/knowledge.md) | Attachments, saved details, and sources |
| [Teams and artifacts](docs/teams-and-artifacts.md) | Helpers and saved work |
| [Schedules](docs/schedules.md) and [Groups](docs/groups.md) | Recurring work and sharing |
| [Connections](docs/connected-services.md) | MCP tools and service setup |
| [Documentation index](docs/README.md) | All guides and technical references |

## Contributing and support

Bug reports, feature requests, documentation improvements, and code contributions
are welcome. Search [existing issues](https://github.com/Sovra-AI/SovraHarness/issues)
before opening a new one, and follow [CONTRIBUTING.md](CONTRIBUTING.md) when
submitting a pull request. Include your app version, OS, and reproduction steps;
remove credentials and personal data from logs and screenshots.

Follow the [Code of Conduct](CODE_OF_CONDUCT.md) in project spaces. Report
vulnerabilities using [SECURITY.md](SECURITY.md), without posting exploit details
in public issues.

## Shipping

[cargo xtask dist](docs/releasing.md) builds installers under `target/dist/`.
The release workflow publishes executables and the Velopack feed when a push to
`main` bumps the package version; a matching `vX.Y.Z` tag can run the same workflow.
See the [release guide](docs/releasing.md) for versioning, signing, and verification.

## License

[LICENSE](LICENSE) is Sovra's Sustainable Use License, Version 1.0, with Sovra
as the licensor. This is a fair-code license with use and distribution
restrictions, not an OSI open-source license. It applies to source on `main`;
the license excludes other branches. Files with `.ee.` in their name or `.ee` in their directory name
are covered separately by [LICENSE_EE.md](LICENSE_EE.md). Third-party components
retain their own licenses.
