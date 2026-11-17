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
are optional and must be supplied together. Press Enter in the password box to
sign in or finish creating the account. A name-only account can be reopened
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
Install Ollama once; Sovra starts its local server when you test, refresh,
download or use a model. Other local programs need their server started
separately. See [local model setup](local-models.md) for presets and troubleshooting.
A local chat model alone does not disable web search, remote
tools or group networking; see [network destinations](network.md#network-destinations).

## Keep a company decision

Open **Artifacts → Decisions → New decision**, or choose **Record a decision**
under a completed chat reply. Add the question, your company's criteria, quoted
evidence, a review date and what would end the decision, then save the draft.

A second person signs in with a different local account to review and accept
it. Use **Sign out**, then **Create another account** if that person is new here.
The record becomes the company position only after **Confirm acceptance**.
Edits retain earlier versions and need fresh acceptance. See
[Decision records](decisions.md) for hard stops, review dates and the limits of
local-account approval.

## Train your own small AI

Open **Training** and press **Start training**. The sample text, a model size
suited to this computer and the **Balanced** preset are already chosen, and no
chat provider or API key is needed. A chart and a sample of the model's own
writing show it learning. When it finishes, find it under **Your models** and
press **Try it**.

If you are in a group, **Train with my group** lets group computers that turned
on **Help the group train** join in, and gives everyone in the group the
finished model. The [Training guide](training.md) covers your own text, the
group steps, what is shared, and the limits. These are small models that write
in the style you taught them; they do not change the AI answering your chats.

## Find the right feature

| I want to… | Start here |
| --- | --- |
| See or change your chat AI | [My AIs](models.md#my-ais) |
| Attach a document, picture or recording | [Files in chat](files.md) |
| Read or search a folder | [Skills and file knowledge](skills-and-file-knowledge.md) |
| See what Sovra remembers and where it came from | [Reviewing knowledge](knowledge.md) |
| Ask several helpers together or keep a reply | [Teams and artifacts](teams-and-artifacts.md) |
| Keep a decision, its evidence and acceptance | [Decision records](decisions.md) |
| Run something later | [Schedules](schedules.md) |
| Share a detail with family or colleagues | [Groups](groups.md) |
| Train your own small AI, alone or with my group | [Training](training.md) |
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
