# Limits and objections

What Sovra does not do today, the objection each limit raises, and the
factual answer. Regulated buyers buy evidence; this list is what a risk team
would find anyway.

Labels: **Today** (ships), **Next** (stage 2, build-ready), **Direction**
(stages 3–4, outline). See [05 — Enterprise vision](05-enterprise-vision.md).

## Product limits

| Limit | Objection | Answer |
| --- | --- | --- |
| **English interface only**; no Arabic screens or right-to-left layout. | "Our staff work in Arabic." | True today. People can write to helpers in any language the chosen model handles; the interface itself is English. Localisation is not on the published roadmap yet and should be added before an Arabic-first rollout. |
| **No Arabic model bundled or labelled.** | "We want UAE-built AI." | Sovra reaches any model through a local server (Ollama or OpenAI-compatible). Jais, Falcon Arabic and K2 Think are open-weight and can be served that way, but have **not been tested** in Sovra. |
| **First setup leads with a cloud key** (OpenRouter); local models are a separate path. | "Out of the box it is not sovereign." | Correct. A fully local setup is possible with shipped options ([03](03-sovereignty-evidence.md), "Fully local setup"), but it is not the default path. |
| **Memory extraction uses the chat model.** | "Does remembering send our chats out?" | Only if the chosen chat model is remote. With a local model it stays on the computer. Remembering can be switched off. |
| **Remembering is automatic by default** — no per-fact approval. | "We want control of what is kept." | Each fact must quote its source; any fact, chain or whole source can be stopped; nothing is deleted. Remembering can be switched off. Per-fact approval is not built. |
| **Search by meaning** needs an embedding model; off by default. | "Is it weaker without it?" | Word search always works. Meaning search runs wherever the person points it — local (Ollama) or remote — and the destination is shown. A bundled local embedding model is groundwork item P7. |
| **No app-level encryption at rest.** | "Stolen laptop?" | Today it relies on the operating system's disk encryption and keychain. Encryption at rest by default is a stage 4 control. |
| **No audit log per user; no agent action log.** | "We need an audit trail." | Today: never-rewritten memory history, artifact versions, send receipts. Per-user audit and agent action logs are stage 4 controls; OCSF export is stage 3. |
| **No SSO, MFA or central policy.** | "IT must control it." | Today each person's OS sign-in and settings apply. Managed identity and IT-signed policy are stage 3 (phase A0). |
| **No certifications** (SOC 2, ISO 27001, ISO 42001). | "Procurement requires them." | Not yet. The roadmap order is SOC 2 Type II → ISO 27001 → ISO 42001. Suitable for pilots that do not require certification. |
| **No verified air-gapped edition.** | "Our network is cut off." | Local models and the offline router work without internet, but a tested air-gapped edition with offline signed updates is stage 4 (phase B2). |
| **No self-update; signing not in every release.** | "How do we patch?" | Velopack signed updates are planned (groundwork P7); today updates are reinstalled. |
| **Schedules are not persisted across restarts.** | "Will jobs survive a reboot?" | Not yet; persisting schedules is groundwork item P5. |
| **Scheduled messages are approved when created, not when they fire.** | "Could a timed message go out unchecked?" | The person approves the recipient, app, exact words and time at creation; a bare first name is refused. There is no second review at send time. |
| **Add-on (MCP) tools**: no approval per call; programs not sandboxed. | "Third-party tools are a risk." | Programs need **Run it** first and re-ask on any change; each tool can be switched off. Per-call approval is not built — keep add-on tools off in sensitive pilots. |
| **Google**: Gmail and Drive capped at 100 users with an "unverified app" warning until Google verification. | "Can we roll out Gmail to everyone?" | Not until verification completes. Gmail is drafts only and Drive read-only regardless. |
| **Each computer works alone.** | "We need shared knowledge." | Sharing across a group of computers is **Next** (stage 2), build-ready. |
| **No company model.** | "We want our own AI." | **Direction** (stage 3): only if evaluation shows it is needed; federated training on the organisation's computers with lineage and rollback. |

## Business objections

| Objection | Answer |
| --- | --- |
| "We already have a sovereign cloud and national models." | Sovra is the layer on the desk. It can use a UAE-hosted or local UAE-built model as its guest, and keeps memory, files and approvals on the organisation's computers. It complements sovereign infrastructure. |
| "Show us the ROI." | We do not quote one. We run a pilot with your baseline and your measurements ([06](06-measuring-value.md)). |
| "AI pilots never scale." | The pilot is built to end in a decision, with stop criteria agreed up front. Scaling to a group of computers is Next; to a company is Direction. |
| "Local AI is less capable than the best cloud models." | Often true for the largest tasks. The choice is per helper and per organisation: local for sensitive work, a chosen cloud for the rest, with that choice made visibly. |
| "Who supports it?" | Not covered in this folder; agree it in the success plan. |

## How limits are communicated

Following the roadmap principle "honest about limits": every deck carries a
**Not yet** slide, and every document in this folder labels Today, Next and
Direction. A limit is never hidden to win a pilot.
