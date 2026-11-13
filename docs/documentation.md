# Maintaining documentation

Documentation describes behavior present in this checkout. Update the relevant
guide in the same pull request as a behavior change, and keep the README concise.
Record release history in [CHANGELOG.md](../CHANGELOG.md), not in dated appendices
to user guides. Track proposals and future work in GitHub issues.

## Choose the right guide

Use the [documentation index](README.md) to find an existing home before adding a
page. Keep first-use instructions, task guides, technical reference, and design
explanations distinct. A task guide should state prerequisites, concrete steps,
what success looks like, and relevant failure cases. Link to implementation
explanations instead of putting them between the steps.

This structure follows [Diátaxis](https://diataxis.fr/start-here/). Contributor
entry points follow [GitHub's contribution-guideline guidance](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/setting-guidelines-for-repository-contributors).

## Write from evidence

- Verify commands against xtask, manifests, scripts, or workflow files.
- Verify visible labels against the UI and behavior against runtime handlers and tests.
- Give exact environment names, defaults, storage locations, and restart requirements.
- Distinguish a library API from a feature wired into the desktop app.
- State relevant limitations without promising future features or delivery dates.
- Use plain language, descriptive headings, short paragraphs, and relative links.
- Keep examples synthetic. Never commit keys, personal logs, machine paths, or
  local MCP descriptors.
- Avoid copying version tables and limits across guides. Link to the owning
  reference or source where duplication would drift.
- Keep measured results with their test conditions; a synthetic test is not a
  product accuracy, performance, or security guarantee.

## Verify a change

`cargo xtask check-docs` checks relative Markdown file targets in root documents
and recursively under `docs`, `.github`, `crates`, and `tests`, plus the required
architecture diagrams and README logo. It does not validate heading anchors or
external URLs. Check those when moving sections or changing links.

Run `cargo xtask check-docs` and `git diff --check` while editing, then
`cargo xtask ci` before submitting. Review Markdown and Mermaid rendering on
GitHub as well: passing a source check does not prove a diagram renders or an
external service is reachable. Run documented commands when feasible and report
checks that require unavailable platforms, credentials, or optional services.

For changes to app behavior, follow the native verification rules in
[AGENTS.md](../AGENTS.md). A documentation-only correction does not require
launching another app instance.
