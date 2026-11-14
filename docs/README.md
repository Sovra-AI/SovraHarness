# Documentation

SovraHarness is a native desktop workspace for AI chat, files, memory, shared
work, and training your own small AI across your group's computers. These guides describe the
current checkout. For a released version, read
the documentation at that release's tag. Start with the [project README](../README.md)
for installation and the [changelog](../CHANGELOG.md) for release history.

## Start here

- [Getting started](getting-started.md): launch Sovra, connect a model, and send a first message.
- [Train your own small AI](training.md): teach a model from your own text in one app, let your group's computers help, watch it learn, and have the finished model delivered to everyone in the group.
- [Models](models.md): choose a provider, check credentials, and change your chat model.
- [Local model servers](local-models.md): automatic Ollama startup, alternative-server presets, official API references and troubleshooting.
- [Configuration and local data](configuration.md): settings, environment variables, data paths, and troubleshooting.

## Use Sovra

| Task | Guide |
| --- | --- |
| Discover capabilities and create reusable instructions | [Skills and folder knowledge](skills-and-file-knowledge.md) |
| Read documents, images, or recordings | [Files in chat](files.md) |
| Review and correct saved knowledge | [Knowledge](knowledge.md) |
| Work with helpers and save their output | [Teams and artifacts](teams-and-artifacts.md) |
| Run prompts or send messages later | [Schedules](schedules.md) |
| Share selected work across computers | [Groups](groups.md) |
| Train your own small AI, alone or with your group | [Training](training.md) |
| Connect WhatsApp or Telegram | [Messaging](communications.md) |
| Use Gmail drafts, Calendar, Drive, Contacts, or Tasks | [Google](google.md) |
| Work with repositories and issues | [GitHub](github.md) |
| Add external tools | [MCP servers](connected-services.md) |
| Allow local commands | [Terminal](terminal.md) |

## Develop and test

- [Contributing](../CONTRIBUTING.md): issues, branches, pull requests, and required checks.
- [Native development](development.md): prerequisites, manual launch, and build caches.
- [Development MCP](development-mcp.md): inspect and operate the native app for testing.
- [Styling](styling.md): native tokens, accessibility, and responsive layout.
- [Releasing](releasing.md): packaging, signing, and release verification.
- [Memory evaluation](agent-memory.md) and [retrieval benchmarks](knowledge-benchmark.md): optional measurement tools and what their results mean.
- [Documentation maintenance](documentation.md): writing and checking these guides.

## Understand the implementation

- [Architecture](architecture.md): crate boundaries and the Command/Event protocol.
- [Agent loop](agent-loop.md): delegation, limits, and streamed progress.
- [Memory capture and recall](memory-capture-and-recall.md): evidence, extraction, retrieval, and withdrawal.
- [File understanding](file-ontology.md): classification and bounded folder scanning.
- [Phone control protocol](remote-control.md): desktop transport and reference client; phone apps live in separate repositories linked from the project README.
- [Swarm internals](swarm.md): how training works inside: the model shapes, work packets and merge rules, saved-file formats, group messages, model delivery and tests.
- [Network access](network.md): configured destinations and local-operation limits.
- [Dependency license evidence](dependency-licenses.md): version-specific clarification.

See [Security](../SECURITY.md) for vulnerability reporting and
[Code of Conduct](../CODE_OF_CONDUCT.md) for participation expectations.
