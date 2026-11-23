# Roadmap

This is the only list of work that is still open. It has no delivery dates.
Guides describe what this checkout does today. When a guide finds a gap, it
states that limit and this page is the place that records the work.

Shipped behavior stays in the [changelog](../CHANGELOG.md) and the guide for
that feature. A **Limits** section is current behavior. It is copied here only
when a guide says the piece is not built, not on the screen, not measured, or
not emitted.

The [latest desktop release](https://github.com/Sovra-AI/SovraHarness/releases/latest) has the installers for macOS, Windows and Linux.

## Company records

[Issue #11](https://github.com/Sovra-AI/SovraHarness/issues/11) tracks these records. Two parts have shipped.

| Part | Where it stands |
| --- | --- |
| Dated answers from the company's files | Shipped in v0.7.0. On Your folders, **Save this answer** keeps the file, the lines, the wording, and the day that version was read. |
| A decision record | Shipped in [v0.8.0](https://github.com/Sovra-AI/SovraHarness/releases/tag/v0.8.0). Open **Artifacts → Decisions** or choose **Record a decision** under a chat reply. A second local account accepts the saved version. |
| A register of outside parties | Open. One page per vendor, partner, tool, or service, with the same headings every time. |
| Separate pieces of work, and a check before new work starts | Open. Knowledge stays inside the customer, deal, project, or team. Before new work starts, say whether the company already holds the other side. |
| A pack that shows the working | Open. One saved pack: which computer, which model, which documents, who accepted the answer, and that the history is still kept. The company chooses the headings. |
| A language of record, with a second language beside it | Open. |
| A recovery plan, or a plan for stopping a product, tied to the live decisions and outside parties | Open. |

Nothing in that issue is filed or treated as final until a person saves it
and, where the record requires it, a second person accepts it.

## Groups

From [Groups](groups.md).

- **Viewer and Contributor on the screen.** The membership fold already refuses additions from a Viewer. No control sets either role.
- **Correct a shared detail from the screen.** `GroupCommand::Correct` and the dispute fold exist. The screen has no correction button, and Share does not set `replaces` or `valid_at`.
- **Join while the inviting computer is off.** Only the computer that made an invite can admit its holder, so it has to be on when they join. The joining computer keeps trying; no other group computer can answer for it.
- **Relays Sovra runs.** Groups across the internet use n0's public relays and lookup service. There is no setting for a relay of your own, and no test across two separate networks: the relay-only tests run both computers on one machine.
- **Say when the group key cannot be stored.** The words for a missing keychain exist on `GroupProblem::NoKeychain` and are not emitted. The screen stays on "Getting your group ready…".

## Training

From [Training](training.md) and [Swarm internals](swarm.md).

- **Bring a Large run to the group.** Chunked work transfer is not built, so Large trains on one computer. The group can still receive the finished model with **Share with my group**.
- **Measure whether extra computers make a run faster.** No speed-up from adding group computers has been measured, and there is no test across separate machines on different networks. Tests run several runtimes in one process.
- **Import or fine-tune a pretrained model.** The trainer teaches the four built-in starting points only. Tokenizer files and an existing language model cannot be brought in. Owner decision (2026-11-11): on the future training screen the person running Sovra picks the open base model, with a licence that allows fine-tuning, and Sovra does not choose a default. Which items may train it, such as an "OK to train on" mark per department, is still to be decided. Until that screen exists, no shared model is trained in the app.
- **Review the unused privacy protocols before any use.** Noise-based privacy and secure aggregation exist as library code and are not applied to desktop training. Swarm internals say that review is required before deployment.

## Files, plugins, and connections

- **A hard deadline for the document decoder.** The configured decoder timeout is not enforced without Xberg's tokio-runtime feature. Cancellation is cooperative. [Files in chat](files.md).
- **Plugin versions beyond `major.minor.patch`.** Prerelease and range syntax are not accepted. [Plugins](plugins.md).
- **Microsoft work this connection does not do.** Live spreadsheet editing, Office automation, Power BI, Dynamics, Azure administration, and Purview. [Microsoft 365](microsoft-365.md).
- **A GitHub Enterprise Server address on the screen.** `sovra_github::Endpoints` supports one in code. The screen does not offer it. [GitHub](github.md).

## Install and phones

- **In-app update checks.** Installer hooks exist. The app does not check the Velopack feed. A push that publishes executables does not add this check. [Releasing](releasing.md).
- **Store releases of the phone apps.** Pairing in this repository talks to [SovraiOS](https://github.com/Sovra-AI/SovraiOS) and [SovraAndroid](https://github.com/Sovra-AI/SovraAndroid), built from source. No store release is published. That work is in those repositories, not in this one.
