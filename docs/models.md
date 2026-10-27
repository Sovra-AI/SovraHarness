# Model library

Start in **Models**. The setup screen uses a state-aware 2-step stepper (`1 · Connect an AI service` → `2 · Pick your AI` → chat) that only shows what is still needed for your saved setup:

1. **Connect OpenRouter** opens the guided account-key form (with a 1-click **Get Free Key at openrouter.ai ↗** browser button). Paste and save your connection key; release builds store it in the OS credential store (`keyring`), and a masked preview (`sk-or-••••1234`) stays visible so you can verify, test, replace, or remove it anytime.
   Sovra checks a new OpenRouter key against `auth/key` before saving it (the model list itself is public, so it cannot prove a key works); a refused key is not saved. Saving the key from setup moves straight on to step 2 (or to Chat, when a model was already chosen).
2. **Pick your AI**: Once a key is connected, step 1 collapses into a confirmation line (`✓ OpenRouter is connected`) and the setup screen, titled **Pick a chat model**, shows up to three suggestions derived from live catalog data: **Free** (a zero-price chat model), **Everyday** (the newest paid model that supports tools and costs at most about 3¢ per typical message) and **Thinks longer** (the same for reasoning models, up to about 6¢). Each shows an estimated cost per message (~1,500 tokens read, ~500 written). Unknown prices and capabilities never qualify. **Use this model** opens the conversation as soon as the runtime confirms the choice; a message typed before setup finished is sent automatically.

If the saved key is missing from this computer's credential store, or the chosen model can no longer run (for example, it left the catalog), setup says so and asks for the key or a new model. It never reports "ready" for a choice that cannot answer. The **Take a tour** button starts the 5-step walkthrough at any time.

**Browse all models** opens the full library. Every row in the catalog includes a direct **Use Model** / **✓ Active** / **Add Key** button on the right for 1-click activation, while clicking the row title opens the inspection panel. Filters start closed; open them when
you need specific capabilities. **Setup** returns to the guided screen. A saved
selection is shown by name, and an empty selection is never described as a working
default AI. Local/other connections are optional, separate paths; the online path
requires no extra installed app.

Use **Models → Browse all models** to browse OpenRouter, installed Ollama models and
your own local or remote connections. The library refreshes in the background;
saved results remain visible when a server is offline. **Refresh** requests current
metadata. Connection cards identify live, cached, loading and failed discovery.

## Find and choose a model

The setup flow follows [recognition rather than recall](https://www.nngroup.com/articles/recognition-and-recall/):
actions and explanations appear together, while technical filters stay in the full
library. Free model availability and limits come from the live catalog and
[OpenRouter's pricing documentation](https://openrouter.ai/pricing), checked on
2026-10-24. Suggestions use listed prices and advertised text/reasoning support;
they do not rank answer quality or infer speed from a model name. Unknown prices
remain visible as unknown and never qualify for the free starting choice.

OpenRouter discovery requests `/api/v1/models?output_modalities=all`. This includes
the complete public catalog, including non-chat outputs; the default API response
would omit those. No account preference or hand-maintained model list narrows it.
The public catalog can be browsed without an API key.

The library lists **chat models** by default and labels every model by what it makes:
chat, pictures, video, voices (text to speech), speech to text, search helpers
(embeddings) or other specialist tools. The **Show** picker beside the search box lists
any other type, or every type at once.

Search names, exact IDs, creators, descriptions and connection names. Combine
provider tabs, a connection, creator, input modality, minimum context,
free/paid usage, tool calling, reasoning and structured output. Modality options
come from the catalog, including future values. Sort by name, release time,
context or reported token price. Unknown prices remain unknown; they are never
treated as free. Lists show an estimated cost per typical message; details show
exact per-million-token prices, and other pricing dimensions retain their provider
labels and units.

Favorites persist across launches. Open a row for its description, context/output
limits, modalities, capabilities, pricing and available provider metadata. Copy
its exact ID or compare up to three models. At small window sizes, details and
expanded filters use separate panels to preserve usable controls. Lists are
virtualized rather than rendering hundreds of rows at once.

**Use model** selects a text-producing model for subsequent chat requests and
persists the choice. A missing credential opens its connection settings. Models
advertised exclusively for images, video, embeddings, transcription or other
non-chat outputs remain browsable but cannot be selected as the chat assistant.
The catalog is not an image/video generation interface. Servers that do not
advertise capabilities can be tried explicitly; the server ultimately determines
what the model supports. Switching models starts fresh provider history.

## Connections

Open **Connections → Add connection** (or **Settings → Advanced → Show advanced
options**), choose a preset and adjust its address. Service type (protocol),
how the key is sent (authentication), wait time and other headers sit behind
**Show technical options**. Use the API base, not a web dashboard URL. Reverse-proxy path
prefixes are preserved. Common full endpoint suffixes are normalized.

| Server | Base URL example | Protocol | Authentication |
| --- | --- | --- | --- |
| OpenRouter | `https://openrouter.ai/api/v1` | OpenRouter | Bearer API key for chat |
| Local Ollama | `http://localhost:11434` | Ollama | None by default |
| Ollama hosted | `https://ollama.com` | Ollama | Bearer API key |
| LM Studio | `http://localhost:1234/v1` | OpenAI-compatible | None, or the server's configured auth |
| vLLM | `http://localhost:8000/v1` | OpenAI-compatible | None, or Bearer when configured |
| llama.cpp server | `http://localhost:8080/v1` | OpenAI-compatible | None, or Bearer when configured |
| Compatible remote API | `https://your-server.example/prefix/v1` | OpenAI-compatible | None, Bearer, or a named API-key header |
| Anthropic Messages | `https://api.anthropic.com` | Anthropic | `x-api-key` |

Compatible servers use `/models` for discovery and `/chat/completions` for chat.
The preset cannot guarantee compatibility with every gateway: an endpoint that
only implements Responses, Gemini, Azure deployment-specific routes or OAuth
needs its matching adapter. Optional OpenAI organization/project headers and a
custom API-key header name are supported. Request/chat timeout is configurable
between 5 and 600 seconds.

**Check connection** verifies discovery and, for OpenRouter, the key endpoint; it
does not send a paid inference request. Saving is also possible when discovery
is unsupported. Use **Add a model by name** on that connection to supply an exact
deployment/model ID, optional context and explicit tool capability. A manual
entry overrides discovered metadata for the same ID. Removing it restores normal
discovery. Removing a connection never deletes models from its server.

Keys are masked in forms and stored through the operating system credential
store (`keyring`); configuration and cached catalogs contain no API keys. A blank
key keeps the saved key. A replacement overwrites it; **Clear the saved key**
removes the saved override. Save errors keep the draft available for correction.
Changing protocol, destination or authentication requires re-entering or clearing
an existing credential. HTTP redirects are disabled to prevent credential
forwarding. Environment keys `OPENROUTER_API_KEY` and `OLLAMA_API_KEY` apply only to
their built-in connection identity, protocol and default URL. A saved key takes
precedence; clearing it can reveal the environment fallback.

## Ollama

Run your Ollama server separately. Discovery uses `/api/tags` and bounded
`/api/show` requests for context, model family, parameter size, quantization and
capabilities. Advertised maximum context can exceed the server's configured
runtime context. A detail lookup failure does not hide an installed model.

In the Ollama tab, select a server and enter an exact model tag to download.
Downloads occur on that server and can be several GB. Progress streams into the
Models view; successful completion refreshes its catalog. Cancel closes the
download connection. Ollama may keep partial layers for a later retry. Editing
or removing the destination also cancels its in-flight download.

## Storage and implementation

Metadata/preferences live under the OS app-data directory in `models/`, with a
`connections.json` file and per-connection `catalogs/` files. Writes replace files
atomically; Unix files are created with mode `0600`. Fresh OpenRouter cache is
reused for up to 15 minutes on startup; explicit Refresh always fetches again.
Other connections are rediscovered on startup. Failed refreshes retain the last
successful result and an error status. Full catalog responses are bounded at
32 MB; Ollama progress records at 1 MB.

`sovra-core::models` owns the protocol, filters and reducer. `sovra-providers`
owns HTTP, keychain and metadata storage behind `ModelProvider`;
`sovra-runtime` owns background tasks, cancellation and selected execution;
`sovra-adk` configures provider clients; `sovra-ui` renders native controls.
No network or keychain work runs on the UI thread. Development still uses the
persistent [tmux live-reload session](development.md).

## Primary references

Verified 2026-10-24 and against pinned dependencies:

- [OpenRouter model catalog](https://openrouter.ai/docs/api/api-reference/models/get-models):
  the `all` modality parameter and model/pricing metadata.
- [Ollama model details](https://docs.ollama.com/api-reference/show-model-details)
  and [authentication](https://github.com/ollama/ollama/blob/main/docs/api/authentication.mdx).
- [LM Studio OpenAI compatibility](https://lmstudio.ai/docs/developer/openai-compat).
- [vLLM OpenAI-compatible serving](https://github.com/vllm-project/vllm/blob/main/docs/serving/online_serving/openai_compatible_server.md).
- [llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server).
- [Anthropic model discovery](https://platform.claude.com/docs/en/api/models/list):
  pagination and advertised capabilities/limits. The pinned ADK integration uses
  the supported `2023-06-01` protocol; optional newer metadata is consumed when present.
- [keyring 4.2](https://docs.rs/keyring/4.2.0/keyring/): OS credential storage.

Discovery success establishes connectivity and catalog access, not inference
permission, billing balance or support for every advertised feature. Tool use is
enabled only when reported by the catalog or explicitly declared for a manual model.

## Native agent UI

Research checked on 2026-10-24: [AG-UI](https://github.com/ag-ui-protocol/ag-ui)
is an interaction protocol, not a desktop widget toolkit. The standalone
[Rust ag-ui SDK](https://docs.rs/ag-ui/latest/ag_ui/) offers protocol/client/server
types; it does not supply GPUI components. For this ADK-based harness,
[ADK-UI](https://adk-rust.com/en/docs/tools/ui-tools) is the best fit because its
Rust toolset already generates structured UI and an AG-UI surface event profile.
The implementation is verified against the pinned `adk-ui 2.2.0` source.

Sovra requests `ag_ui` output from the card, confirmation, table, alert and progress
tools, decodes their `adk.ui.surface` CUSTOM events, and translates the embedded
component tree to pure domain types. GPUI renders these natively without a WebView.
Repeated revisions replace the current surface. Buttons send a JSON action with
surface ID, revision and action ID as the next prompt in the same session.
Only tools with native renderers are advertised; browser forms, charts, 3D scenes,
and arbitrary remote AG-UI HTTP endpoints are not currently exposed.
