# Getting started with Sovra

Sovra is a native desktop workspace for chatting with helpers, remembering useful
details and working with your files. Your choice of model determines where chat
content is processed: a cloud connection sends it to that provider; a local
model runs on its configured computer.

## Launch and sign in

For a source checkout, follow [Native development](development.md) to install
the build prerequisites and run `cargo xtask dev`. Installed builds can be
launched normally. The interface defaults to Arabic; Settings includes a language
picker and **Text size** from 75–200%.

On a fresh workspace, the email/password form creates the first local account.
Later launches restore the session or ask you to sign in. Google sign-in needs a
configured desktop OAuth client; see [Google](google.md). Local accounts do not
establish separate encrypted data stores or company SSO. The current sign-in
limits are documented in [development](development.md#sign-in-integration-status).

## Send your first message

1. Open **Models** and connect an AI service. The guided path offers OpenRouter;
   local servers and compatible services are also supported.
2. Choose a chat model and wait for the saved selection to be confirmed.
3. Open Chat and ask a simple question. A successful connection check alone
   does not prove that the provider allows inference or has available credit.
4. Open **Skills** to find capabilities in plain words and try an example.

The [Models guide](models.md) explains keys, discovery, costs and capabilities.
For local models, start the model server separately; Sovra does not install or
launch it for you. A local chat model alone does not disable web search, remote
tools or group networking; see [network destinations](value-realisation/03-sovereignty-evidence.md#2-every-network-destination).

## Find the right feature

| I want to… | Start here |
| --- | --- |
| Inspect routing and effort choices | [My AI's and routing](models.md#my-ais-and-routing) |
| Attach a document, picture or recording | [Files in chat](files.md) |
| Read or search a folder | [Skills and file knowledge](skills-and-file-knowledge.md) |
| See what Sovra remembers and where it came from | [Reviewing knowledge](knowledge.md) |
| Ask several helpers together or keep a reply | [Teams and artifacts](teams-and-artifacts.md) |
| Run something later | [Schedules](schedules.md) |
| Share a detail with family or colleagues | [Settings → Groups](groups.md) |
| Connect WhatsApp or Telegram | [Messaging](communications.md) |
| Use Google services | [Google](google.md) |
| Add tools from another service | [MCP servers](connected-services.md) |
| Let a helper run a computer command | [Terminal](terminal.md) |

Memory extraction happens in the background, so a newly stated fact may only be
available on a later turn. Check its source before relying on it. Save useful
replies and code blocks to Artifacts; editing saved work keeps versions.

For missing keys, data locations and startup settings, see
[Configuration and local data](configuration.md). For the full documentation map,
see [the docs index](README.md).
