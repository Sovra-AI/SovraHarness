# The sovereignty model

What "sovereign AI" means for an organisation using Sovra, broken into seven
layers, with where Sovra stands on each today and what comes next. The
evidence behind every **Today** line is in
[03 — Sovereignty evidence](03-sovereignty-evidence.md).

## Why seven layers

Most "sovereign AI" claims stop at **where the data is stored**. For an
organisation that plans to have AI do a large share of its work, that is not
enough. It also matters who holds what the AI *knows*, whose AI it is, who
approves what it *does*, who owns what it *learns*, who controls the software
itself, and which jurisdiction all of that sits in.

| # | Layer | The question it answers |
| --- | --- | --- |
| 1 | **Data** | Where do our files, messages and records live? |
| 2 | **Knowledge** | Who holds what the AI has learned about our work, and can we check and correct it? |
| 3 | **The model** | Whose AI answers, where does it run, and can we change it? |
| 4 | **Decisions** | Can the AI act without a person approving? |
| 5 | **Learning** | Does our work train someone else's AI? Can we train our own? |
| 6 | **Operations** | Who controls updates, telemetry and the software supply chain? |
| 7 | **Jurisdiction** | Can we prove all of the above stays in our country? |

## Layer 1 — Data

**Today.**

- Sovra is a native program on the person's computer. Its memory database,
  file index, logs, attachments, schedules and settings live in the app's data
  folder on that computer.
- Files are read **in place**; removing a folder deletes Sovra's index, never
  the files.
- Keys and connection secrets live in the operating system's keychain.
- There is **no telemetry**: no analytics or crash-reporting service exists in
  the code. Sovra-the-company receives nothing.

**Caveats that are true today.**

- Content goes wherever the person points it: a cloud model, web search,
  Google, WhatsApp, Telegram or an add-on tool each receive what they need.
- A fresh install has an OpenRouter connection by default and refreshes its
  public model list at launch (no user content). Web search is on by default
  and sends search queries to Mwmbl and Wikipedia.
- These are **off switches**, listed in [03](03-sovereignty-evidence.md).

**Gap.** Encryption at rest is left to the operating system (stage 4 control).

## Layer 2 — Knowledge

**Today.**

- Memory is captured from the person's private chats. Every remembered fact
  must quote the **exact words** it came from, or the whole batch is
  rejected.
- Facts are **bi-temporal** and **never edited or deleted**. A correction
  supersedes the old fact, and the old fact stays as history. The helper can
  answer "what did we believe, and when?".
- Any fact, correction chain or whole source can be **stopped**.
  Remembering can be switched off entirely.
- Chats from connected accounts and messages from strangers are not
  remembered.
- File knowledge uses open vocabularies (DCMI, schema.org, NEPOMUK, IPTC,
  PROV-O). AI-proposed links between files are saved only when a person
  presses **Save**.

**Caveat.** Extraction uses the person's chosen chat model. With a cloud
model, the message being remembered goes to that provider.

**Gap.** [Groups](../groups.md) can share explicit text details. Private encrypted
**lanes** across a group remain **Next** (stage 2). Per-fact approval before
private memory capture is not built.

## Layer 3 — The model

**Today.**

- The model is a **replaceable guest**. Sovra connects to models on the same
  computer (Ollama, or any OpenAI-compatible server such as LM Studio,
  llama.cpp or vLLM), or to a cloud the person chooses (OpenRouter, Anthropic,
  any compatible API).
- Swapping the model changes nothing else: memory, files, helpers and saved
  work stay.
- Routing between models is decided locally, by a router at a local address
  or the built-in rules, which need no internet.
- UAE-built open models (Jais, Falcon Arabic, K2 Think) can be served
  locally and used this way. They have **not been tested** in Sovra.

**Gap.** First setup leads with a cloud key, and no local model is bundled.

## Layer 4 — Decisions

**Today.**

- **Messages:** WhatsApp and Telegram messages are prepared as review cards
  with the exact words. No model tool can approve or change them. Approval
  expires after ten minutes, and nothing is retried automatically.
- **Email, calendar, drive:** Gmail gets drafts only; Calendar events invite
  nobody; Drive is read-only.
- **The computer:** commands show Allow once / Always allow / Don't allow.
  Risky forms are asked every time and never remembered.
- **Outsiders:** strangers who message the bot get no memory, files, skills,
  connected services or tools.
- **File links:** only a person's **Save** approves an AI-proposed link.

**Caveats.**

- Scheduled messages are approved when the job is created, not again when
  they fire.
- Add-on (MCP) tools have no approval per call yet; they can be switched off
  per tool.

**Gap.** Supervision queues and an agent action log are stage 4 controls.

## Layer 5 — Learning (training)

**Today.**

- Sovra does not train any model, and sends nothing to Sovra-the-company that
  could be used for training.
- With a local model and web search off, the work never leaves the computer,
  so it cannot train anyone else's AI.
- With a cloud model, the provider's own terms govern what happens to prompts.
  The organisation chooses the provider.

**Direction (not today).**

- The organisation trains its **own** small adapters on its own idle
  computers. Original files never leave; only the lessons are combined, with
  secure aggregation and differential privacy.
- Every promoted model is a signed ledger entry with one-step rollback:
  **model lineage**.
- The roadmap is explicit: "Decide whether it is needed before building it",
  starting with federated evaluation ([05](05-enterprise-vision.md)).

## Layer 6 — Operations

**Today.**

- No telemetry, no licence check and no update check at start.
- One Rust codebase with no unsafe code, layered by machine-checked rules.
- Licence and advisory checks (`cargo-deny`) and unused-dependency checks run
  in CI.

**Gaps.**

- The app cannot update itself yet, and signing is not in every release.
- An offline-mirrorable signed update channel, a software bill of materials,
  reproducible builds and an air-gapped edition are stage 4.

## Layer 7 — Jurisdiction

**Today.** Because data, knowledge and (with a local model) inference stay on
the organisation's computers, the work stays wherever those computers are.
There is no Sovra server in another country in the path.

**Gap.** There is no *evidence* product yet. Residency **placement rules**
and a **residency report generated from the ledger** are stage 3 (phase A3).
Residency attestations for sovereign deployments are stage 4 (phase B4).

## The sovereignty scorecard

| Layer | Today | With a fully local setup | Next (stage 2) | Direction (stages 3–4) |
| --- | --- | --- | --- | --- |
| 1 Data | On the device; content goes only where the person points it | Nothing leaves | Encrypted lanes; no central server | Encryption at rest; per-lane cloud kill switch |
| 2 Knowledge | Sourced, never rewritten, stoppable; per computer | Nothing leaves | Shared in lanes, filtered before retrieval | Ledger memory with witnesses |
| 3 Model | Person's choice, local or cloud | Local, possibly UAE-built | Busy laptops hand work to a free computer's local model | IT-signed allowed-model policy |
| 4 Decisions | Approval before send or run (with listed caveats) | Same | Shared helpers only *request* tools | Supervision queues, agent action log |
| 5 Learning | No training; nothing sent to Sovra | Cannot train others' AI | — | The organisation's own AI, with lineage and rollback |
| 6 Operations | No telemetry; no self-update yet | Same | Blind relays the group can self-host | Signed offline updates, SBOM, air-gapped edition |
| 7 Jurisdiction | Follows the computers | Follows the computers | Follows the group's computers | Residency report and attestations |

**What "100% sovereign" can honestly mean today:** a fully local setup on
computers the organisation owns, where layers 1–4 stay entirely inside the
organisation. Layer 5 is satisfied in the negative (nothing trains anyone
else's AI). Layers 6–7 still lack the *evidence* products of stages 3–4.
