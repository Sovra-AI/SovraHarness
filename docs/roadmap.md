# Roadmap

This is the only list of work that is still open. It has no delivery dates.
Guides describe what this checkout does today. When a guide finds a gap, it
states that limit and this page is the place that records the work.

A GitHub issue is the detail for one item here, not a second list. The only
open issue is
[Keep a company's dated answers, decisions, and separate work](https://github.com/Sovra-AI/SovraHarness/issues/11).

Shipped behavior stays in the [changelog](../CHANGELOG.md) and the guide for
that feature. A **Limits** section is current behavior. It is copied here only
when a guide says the piece is not built, not on the screen, not measured, or
not emitted.

The published download is **v0.8.0**. The package version is 0.8.0.

## Company records

Issue **#11**. Two parts are in the product.

| Part | Where it stands |
| --- | --- |
| Dated answers from the company's files | Shipped in v0.7.0. On Your folders, **Save this answer** keeps the file, the lines, the wording, and the day that version was read. |
| A decision record | On `main`, not in the v0.7.0 download. **Artifacts → Decisions** and **Record a decision** under a chat reply. A second local account accepts the saved version. |
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
- **Say when the group key cannot be stored.** The words for a missing keychain exist on `GroupProblem::NoKeychain` and are not emitted. The screen stays on "Getting your group ready…".

## Training

From [Training](training.md) and [Swarm internals](swarm.md).

- **Bring a Large run to the group.** Chunked work transfer is not built, so Large trains on one computer. The group can still receive the finished model with **Share with my group**.
- **Measure whether extra computers make a run faster.** No speed-up from adding group computers has been measured, and there is no test across separate machines on different networks. Tests run several runtimes in one process.
- **Import or fine-tune a pretrained model.** The trainer teaches the four built-in starting points only. Tokenizer files and an existing language model cannot be brought in.
- **Review the unused privacy protocols before any use.** Noise-based privacy and secure aggregation exist as library code and are not applied to desktop training. Swarm internals say that review is required before deployment.

## Files, plugins, and connections

- **A hard deadline for the document decoder.** The configured decoder timeout is not enforced without Xberg's tokio-runtime feature. Cancellation is cooperative. [Files in chat](files.md).
- **Plugin versions beyond `major.minor.patch`.** Prerelease and range syntax are not accepted. [Plugins](plugins.md).
- **Microsoft work this connection does not do.** Live spreadsheet editing, Office automation, Power BI, Dynamics, Azure administration, and Purview. [Microsoft 365](microsoft-365.md).
- **A GitHub Enterprise Server address on the screen.** `sovra_github::Endpoints` supports one in code. The screen does not offer it. [GitHub](github.md).

## Install and phones

- **In-app update checks.** Installer hooks exist. The app does not check the Velopack feed. A push that publishes executables does not add this check. [Releasing](releasing.md).
- **Store releases of the phone apps.** Pairing in this repository talks to [SovraiOS](https://github.com/Sovra-AI/SovraiOS) and [SovraAndroid](https://github.com/Sovra-AI/SovraAndroid), built from source. No store release is published. That work is in those repositories, not in this one.
