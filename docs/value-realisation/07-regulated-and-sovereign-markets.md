# Regulated and sovereign markets

What local-first AI changes for organisations whose data must not leave — and
what it does not change. Includes a section on the United Arab Emirates.

> **Not legal advice.** These are summaries for orientation. Rules move;
> re-check each item in [09 — Sources](09-sources.md) and engage counsel per
> sector and jurisdiction before any sales or compliance claim. The product
> controls described as "to build" are **the direction, not today**
> ([`04-regulated-work.md`](../roadmap/04-regulated-work.md)).

## The core message, stated carefully

Running the AI, its memory and its decisions on the organisation's own
computers **removes whole categories of obligation**
([`04-regulated-work.md`](../roadmap/04-regulated-work.md) §1):

- cross-border transfer paperwork for that work;
- often, the vendor's status as a processor or business associate, because
  the vendor never holds the data;
- a central vendor breach target holding many customers' data.

It **does not remove** the obligations the organisation itself carries:
access control and multi-factor sign-in, audit logs, encryption at rest,
retention and legal hold, eDiscovery and data-subject requests, model risk
documentation, human oversight, staff training, and the organisation's own
risk assessments.

Because there is no vendor cloud to provide those, **they must ship inside
the product**. Sovra today provides some of the foundations (local storage,
approval before action, history that is never rewritten); the rest is on the
roadmap below.

## What helps today versus what must be built

| Obligation | Sovra today | To build (Direction) |
| --- | --- | --- |
| Data stays in the jurisdiction | Memory, files and logs stay on the computer; with a local model the content never leaves ([03](03-sovereignty-evidence.md)). | Lane placement rules and a residency report from the ledger (stage 3, phase A3). |
| Human oversight | Approval before any command, message send or email draft; the model cannot approve itself. | Supervision queues; agent action log. |
| History and accountability | Memory corrections supersede, never erase; saved work keeps versions. | Per-user audit log checked against the ledger; OCSF export. |
| Encryption at rest | Relies on the operating system's disk encryption. | Encryption at rest by default. |
| Identity and access | Each person's own computer and OS sign-in. | SSO / MFA, auto-logoff, directory groups as lanes. |
| Retention, legal hold, data-subject requests | Not built. | Retention schedules, legal hold, access and erasure requests. |
| Cloud use policy | Each user chooses models and connections. | IT-signed policy; per-lane cloud kill switch. |
| No telemetry | No analytics or crash reporting in the code today. | A formal no-telemetry mode with an offline signed licence. |
| Supply chain | One Rust codebase; licence and advisory checks in CI. | Signed updates mirrorable offline (TUF), SBOM, SLSA, reproducible builds, air-gapped edition. |
| Certification | None yet. | SOC 2 Type II → ISO 27001:2022 → ISO 42001:2023 → sector-specific. |

## Sectors at a glance

From [`04-regulated-work.md`](../roadmap/04-regulated-work.md) §2, condensed:

| Sector | Typical rules named in the roadmap | Why local-first helps | What still has to ship |
| --- | --- | --- | --- |
| Health | HIPAA (US); UAE health data law (below) | Patient data need not leave the clinic | Audit log, encryption at rest, access control |
| Privacy (EU/UK) | GDPR | No international transfer for local work | Data-subject requests, erasure, records |
| AI regulation (EU) | EU AI Act, phased dates | Human oversight is built into the product | Model risk documentation, transparency |
| Law | ABA Formal Opinion 512 (US) | Client files stay in the firm | One lane per matter; ethical walls |
| Finance | SEC 17a-4, FINRA, DORA, SOX | Never-rewritten history | Retention, supervision, third-party attestation |
| Government | FedRAMP, CJIS, FIPS 140-3 (US) | No vendor cloud in the path | Air-gapped edition, FIPS-validated crypto |
| National sovereignty laws | India DPDP, China PIPL, Saudi PDPL, UAE PDPL | Data stays in-country by default | Residency attestations (stage 3 A3, stage 4 B4) |

## The United Arab Emirates

### The ambition

- **National Strategy for AI 2031** — global AI leadership as a national goal,
  with the world's first Minister of State for AI.
- **Abu Dhabi** aims to be the world's first fully **AI-native government** by
  2027 under its Digital Strategy 2025–2027.
- **Federal Cabinet, April 2026**: a plan to run half of government sectors,
  services and operations on agentic AI within two years, with AI training for
  federal employees.
- **Dubai Universal Blueprint for AI**: Chief AI Officers in Dubai government
  entities.
- **Adoption**: the UAE ranked first worldwide for AI use among working-age
  people in early 2026.

An AI-native government is one in which a large share of routine work is done
by AI that acts. That raises three sovereignty questions directly: **whose
computers the AI runs on, who keeps what it learns, and who approves what it
does.** Sovra's answer is in [02 — Sovereignty model](02-sovereignty-model.md).

### The rules the data lives under (summaries)

| Rule | What it says, briefly | How Sovra relates today |
| --- | --- | --- |
| **PDPL**, Federal Decree-Law 45/2021, Art. 22–23 | Personal data may be transferred abroad to adequate jurisdictions or under listed exceptions. | With a local model, content stays on the computer; there is no transfer for that work. Cloud models chosen by the user are a transfer the organisation must assess. |
| **Health data**, Federal Law 2/2019, Art. 13 | Health data for services in the UAE may not be stored, processed or transferred outside the UAE except as the health authorities decide; this applies in free zones too. | Same: local model and on-device storage keep the work in the country. Audit log and encryption at rest are still to build. |
| **DIFC** Data Protection Law 2020, Regulation 10 (2023) | Duties for deployers and operators of autonomous and semi-autonomous systems processing personal data. | Approval before action and memory with sources support the transparency and oversight duties; formal documentation is to build. |
| **ADGM** Data Protection Regulations 2021 | GDPR-modelled regime in Abu Dhabi Global Market. | As GDPR above. |
| **TDRA** Information Assurance Regulation | Mandatory for government and designated critical entities. | Controls mapping is to build with certification. |
| **DESC** Cloud Service Provider standard (Dubai) | Security standard for cloud serving Dubai government. | Sovra is not a cloud service; local deployment sits outside that path. A UAE-hosted model provider would need it. |
| **UAE AI Charter** (2024) | 12 non-binding principles, including human oversight, transparency and privacy. | Human oversight is the product's default (see [03](03-sovereignty-evidence.md)). |

### UAE-built AI as the guest model

The UAE has published open models: **Jais** (Inception/G42, MBZUAI), **Falcon
Arabic** and **Falcon-H1 Arabic** (TII), and **K2 Think** (MBZUAI). Because
Sovra treats the model as a replaceable guest reached through a local server
(Ollama or any OpenAI-compatible endpoint), an organisation can choose to run
a UAE-built model on its own hardware. **Sovra has not yet tested or labelled
these models in the app**, and the app's interface is English only today
([08](08-limits-and-objections.md)).

### Positioning in one paragraph

The UAE is building national AI capacity — data centres, sovereign clouds and
its own models. Sovra is the layer that sits **on the desk**: the helpers,
memory, files and approvals that make a public servant or an employee
AI-native, while keeping that work, that knowledge and those decisions on
computers the organisation owns. It complements sovereign infrastructure
rather than competing with it: the model can be UAE-built and UAE-hosted, and
the knowledge never has to leave the building.

## Recommended first verticals (for discussion)

The roadmap leaves "which vertical first" open (§8). Criteria that favour an
early start with today's product:

- the work is document- and message-heavy;
- the data must stay in the country or the organisation;
- a small pilot with named people is possible;
- the buyer does not yet require certification for a pilot.

Small clinics, law firms and advisory practices fit; large regulated
deployments wait for the stage 4 controls and certifications.
