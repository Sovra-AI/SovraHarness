# Getting started with Sovra

Sovra is a native desktop workspace for chatting with helpers, remembering useful
details and working with your files. Your choice of model determines where chat
content is processed: a cloud connection sends it to that provider; a local
model runs on its configured computer.

## Launch and sign in

For a source checkout, follow [Native development](development.md) to install
the build prerequisites and run `cargo xtask dev`. Installed builds can be
launched normally. The interface starts in English; the sign-in screen and Settings include a language
picker and **Text size** from 75–200%.

On a fresh workspace, create a local account with your name. Email and password
are optional and must be supplied together. A name-only account can be reopened
using its named sign-in button on this computer.
Later launches restore the session or ask you to sign in. Google sign-in needs a
configured desktop OAuth client; see [Google](google.md). Local accounts do not
establish separate encrypted data stores or company SSO. The current sign-in
limits are documented in [development](development.md#sign-in-integration-status).

## The guided tour

The first time you sign in, a guided tour opens above each page and walks you
through Sovra in 12 short steps: what Sovra is, giving it an AI, asking a
question, **Skills**, **Knowledge**, **Connections**, the AI **Team**,
**Schedules**, saved work (**Artifacts**), **Groups**, **Training** and **Settings**. Each step
says what the page is for in everyday words and suggests one thing to try.

Step 2, *Give Sovra a brain*, offers two choices with numbered steps:

- **OpenRouter** (online, works on any computer): **Open OpenRouter** opens its key
  page in your browser; make a free account, create a key, copy it, then press
  **Paste my key**.
- **Ollama** (free and private, runs on this computer): **Get Ollama** opens its
  download page; install it, then press **Set up Ollama** to pick a free model.

If an AI already works, the step says so instead. **End tour** leaves at any
point and is remembered; **Take a guided tour** in the menu, or **Take a tour**
in Settings, replays it.

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
tools or group networking; see [network destinations](network.md#network-destinations).

## Try training on your computer

Open **Training**, keep **Try the garden sample** selected, and choose
**Start training**. This runs a tiny practice model inside Sovra without a chat
provider or API key. Wait for saved progress to appear; **Pause** and
**Resume saved training** let you continue later.

To let computers in your group contribute, follow [Training together](training.md).
The preview trains a tiny next-byte model; pretrained SLM fine-tuning is still
being built, and training does not change the AI answering your chats.

## Find the right feature

| I want to… | Start here |
| --- | --- |
| Inspect routing and effort choices | [My AI's and routing](models.md#my-ais-and-routing) |
| Attach a document, picture or recording | [Files in chat](files.md) |
| Read or search a folder | [Skills and file knowledge](skills-and-file-knowledge.md) |
| See what Sovra remembers and where it came from | [Reviewing knowledge](knowledge.md) |
| Ask several helpers together or keep a reply | [Teams and artifacts](teams-and-artifacts.md) |
| Run something later | [Schedules](schedules.md) |
| Share a detail with family or colleagues | [Groups](groups.md) |
| Train a small practice model together | [Training](training.md) |
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
