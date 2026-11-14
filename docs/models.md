# Models and connections

Connect an AI service, choose a chat model, and check where your requests will go.
A catalog entry describes advertised capabilities; only an actual chat verifies
that a service can answer with your account and available credit.

## Connect and choose a model

1. Open **Models**. Use the guided OpenRouter setup or configure a local/compatible
   service from Connections. Sovra starts installed local Ollama servers when
   needed; start other local servers in their own programs.
2. Save a connection key if required. **Check key** checks whether OpenRouter
   accepts it; saving a key and checking a key are distinct actions. A saved key
   is not proof of successful inference.
3. Choose **Next: pick a chat model**, then **Use this model** on a suitable choice.
   Wait for the saved selection to be confirmed.
4. Open Chat and send a short question. If it fails, review the connection error,
   exact model ID, provider permissions, and available credit.

Suggestions use catalog prices and advertised capabilities. Estimates are not
billing guarantees, model-quality rankings, or promises that a free model will
remain available. **Browse all models** opens the full catalog. Cached results
remain visible during refresh or when a server is offline.

## A focused Models home

Once connected, the home shows **Your chat model · Ready**, its name, estimated
message cost and advertised strengths. **Go to chat** is the primary action.
**Change model** reveals a searchable list of ready alternatives; **Cancel change**
closes it without changing your selection. A choice stays pending until the runtime
confirms it. Switching starts fresh provider history, explained beside the choice.
**Models for files (optional)** reveals the existing picture, audio and document
assignments. The tour stays in the page header. **Browse all models** opens the
advanced catalogue, including connections, filters, favourites and comparisons.

## Find and choose a model

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
come from the catalog. Sort by name, release time,
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

## My AI's and routing

**My AIs** shows the chat model chosen in Models and lets you explore routing
previews. Compare your chosen model, preferred effort per helper, and automatic
routing rules. These previews do not switch the model answering chats or check
connection availability; their settings last for the current app session. Expand
**Preview an AI choice** to try a question without sending a chat.

Choose **One AI for everything**, **Choose per task**, or **Automatic**,
with Quick/Balanced/Thorough effort choices.

Routing evaluation and actual provider selection are separate. The current
ordinary-chat runtime emits a route evaluation, then runs the harness built from
the saved Models selection. A route's suggested model or tier does not prove that
a different provider model answered. Use **Models** to change the execution model.
Routing settings and specialist overrides currently live in runtime state and
reset at restart.

The default decision backend is Laya at `127.0.0.1:8011`. The optional OpenJev
backend sends the routing prompt to OpenRouter's decisions endpoint. Failures
fall back to embedded evaluation after a bounded 1.2-second request. Selecting
**Embedded Rust Heuristic** avoids that decision request; merely selecting
**One AI for everything** does not disable the backend request. Route simulation
also contacts the selected backend. See the [network inventory](network.md#network-destinations).

Implementation: `sovra-core/src/routing_tree.rs`,
`sovra-adk/src/decision_client.rs`, `sovra-runtime/src/lib.rs`, and
`sovra-ui/src/my_ais_view.rs`.

## Connections

Open **Connections** and choose **Another compatible service**. Service type (protocol),
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
| llama.cpp server | `http://localhost:9931/v1` | OpenAI-compatible | None, or Bearer when configured |
| Compatible remote API | `https://your-server.example/prefix/v1` | OpenAI-compatible | None, Bearer, or a named API-key header |
| Anthropic Messages | `https://api.anthropic.com` | Anthropic | `x-api-key` |

Compatible servers use `/models` for discovery and `/chat/completions` for chat.
The preset cannot guarantee compatibility with every gateway: an endpoint that
only implements Responses, Gemini, Azure deployment-specific routes or OAuth
needs its matching adapter. Optional OpenAI organization/project headers and a
custom API-key header name are supported. Request/chat timeout is configurable
between 5 and 600 seconds for individual catalog and connection requests. Local
presets default to 600 seconds, and chat turns on loopback Ollama/compatible
connections allow at least ten minutes even with an older saved wait setting.
**Show technical options** also controls streaming and the token-limit field;
the GPT4All preset uses complete replies and `max_tokens`.
See [local model servers](local-models.md) for all 16 alternatives, their official
API references, and version-specific ports and setup requirements.

**Check connection** verifies discovery and, for OpenRouter, the key endpoint; it
does not send a paid inference request. Saving is also possible when discovery
is unsupported. Use **Add a model by name** on that connection to supply an exact
deployment/model ID, optional context and explicit tool capability. A manual
entry overrides discovered metadata for the same ID. Removing it restores normal
discovery. Removing a connection never deletes models from its server.

Keys are masked in forms. Installed/release builds use the operating system
credential store (`keyring`); debug `SOVRA_DEV=1` uses separate development files
([storage details](configuration.md)). Configuration and cached catalogs contain no API keys. A blank
key keeps the saved key. A replacement overwrites it; **Clear the saved key**
removes the saved override. Save errors keep the draft available for correction.
Changing protocol, destination or authentication requires re-entering or clearing
an existing credential. HTTP redirects are disabled to prevent credential
forwarding. Environment keys `OPENROUTER_API_KEY` and `OLLAMA_API_KEY` apply only to
their built-in connection identity, protocol and default URL. A saved key takes
precedence; clearing it can reveal the environment fallback.

## Ollama

Install Ollama once. For a selected local HTTP loopback connection, Sovra checks
the server and starts `ollama serve` if needed. Connection checks, refreshes and
explicit downloads also prepare the server. Startup waits up to 30 seconds and
reports missing installations or startup failures. Remote servers, HTTPS and
reverse-proxy paths remain externally managed. Existing services stay running
when Sovra closes; servers started by Sovra belong to its runtime.

Discovery uses `/api/tags` and bounded
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
No network or keychain work runs on the UI thread. Development uses the
one-shot [native launch](development.md) (`cargo xtask dev`; re-run after edits).

## Native structured results

Supported tools can return cards, confirmations, tables, alerts, and progress
through AG-UI surface events. GPUI renders the supported components without a
WebView. Buttons send their visible label as a follow-up prompt in the same
session. The built-in MCP sample also demonstrates a native card.
Arbitrary remote AG-UI endpoints and browser-only widgets are not exposed.
See [MCP servers](connected-services.md) and [Teams and artifacts](teams-and-artifacts.md).
