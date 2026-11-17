# Downloadable plugins

Sovra's first plugin API bundles reusable instructions and suggested service
connections. A publisher hosts static JSON files, including on GitHub Pages.
Sovra browses catalog metadata, downloads only the selected package, verifies it,
and stores it on this computer. Publishers do not need a running registry server.

## Use a catalog

Open **Skills → Plugins**, paste the full HTTPS address of `catalog.json`, and
choose **Load catalog**. No default publisher is configured. The last successfully
loaded catalog address is saved; leaving the address box empty loads that address.
Use the previous/next buttons to browse pages. The filter searches the current
remote page and all installed metadata; it is not a global remote search.

**Download** installs a plugin switched off. **What's included** shows its skills
and suggested connections. **Turn on** makes its skills available in chat and
under **From plugins** on the Skills page, including automatic selection when the
description matches a request. **Turn off** removes those skills from future
turns; it does not interrupt a turn already using a snapshot of the skill library.

**Set up connection** opens the existing Connections form with the name and
address filled in. The person still chooses to connect and completes that
service's authentication. Connections have their own lifecycle: turning a plugin
off does not disconnect an independently configured service. Stop it in
**Connections**. A package cannot contain credentials, shell commands, binaries,
installation scripts, or permission grants.

Installed packages and their on/off choices work offline after restart. Failed
downloads, invalid packages and incompatible updates leave the installed version
and its choice intact. Updates preserve the on/off choice. There are no automatic
updates, uninstall UI, dependency downloads, or executable plugin API in version 1.

## Publisher contract: version 1

The authoritative parser and types are
[`sovra-core/src/plugins.rs`](../crates/sovra-core/src/plugins.rs).
The [example package](../examples/plugins/packages/weekly-planning-1.0.0.json)
and [example catalog](../examples/plugins/catalog.json) are checked in tests.
Unknown fields, repeated JSON fields, unsupported format versions and invalid
contents are rejected. All strings are UTF-8.

### Package

A package is one self-contained JSON document:

```json
{
  "schema_version": 1,
  "api_version": 1,
  "id": "io.github.your-account.weekly-planning",
  "version": "1.0.0",
  "title": "Weekly planning",
  "description": "Help choosing priorities and planning your week.",
  "skills": [{
    "name": "plan",
    "title": "Plan a calmer week",
    "description": "Turn your tasks into a manageable plan.",
    "instructions": "Ask what needs doing this week. Suggest three priorities and leave room for rest."
  }],
  "services": [{
    "name": "My planning service",
    "address": "https://service.example/mcp"
  }]
}
```

| Field | Contract |
| --- | --- |
| `schema_version` | File format version, currently exactly `1` |
| `api_version` | Host capability contract, currently exactly `1`; catalog entries for other APIs can be displayed but cannot be installed |
| `id` | Stable reverse-domain identifier, at least three dot-separated lowercase name segments, at most 128 bytes; use a namespace you control |
| `version` | Immutable `major.minor.patch`, each an unsigned 32-bit integer without leading zeroes; prerelease/range syntax is not supported yet |
| `title`, `description` | Plain descriptions, required, at most 120 and 1000 bytes respectively; must match the catalog |
| `skills` | Up to 16 procedures; no aliases, actions, tool grants or filesystem resources |
| Skill `name` | Unique within the package; lowercase letters/digits joined with single hyphens, at most 32 bytes |
| Skill `instructions` | Nonempty text, at most 64 KiB, subject to the total package limit |
| `services` | Up to eight named HTTPS MCP addresses, configured separately through Connections |

At least one skill or service is required. Package size is at most 512 KiB.
Names and descriptions also pass Sovra's existing skill validation. Instruction
text remains lower-priority procedure guidance; it does not create capabilities
or bypass a tool's existing approval and access rules.

Skill command names are `plugin-<identity-hash>-<skill-name>`, where the hash is
the first 16 lowercase hex characters of SHA-256 of the plugin ID. Human titles
stay readable. The combined skill catalog checks every command for collisions,
including built-ins and personal skills. Plugin-owned skills are managed through
their package, not overwritten through the personal skill editor.

### Catalog pages

Each page has `schema_version: 1`, a `plugins` array, and optional `next` and
`previous` absolute page addresses (or `null`). An entry contains `id`, `version`,
`api_version`, `title`, `description`, `package_url`, and the lowercase hex
`sha256` of the **exact downloaded package bytes**. Whitespace changes require a
new checksum. Catalog pages contain no instruction bodies.

Each page is bounded to 128 entries and 256 KiB; there is no fixed page count in
the client. Only the current page is kept. Navigation retains the original
catalog address as publisher identity. Multiple catalogs can be used in sequence;
installed plugins remember their own originating catalog.

Addresses must be HTTPS without embedded credentials, queries or fragments.
HTTP loopback addresses are accepted for local development. Downloads reject
redirects, have a 20-second total timeout, and enforce byte limits even when the
server omits its content length. Publish direct JSON addresses.

An update must retain the plugin ID and originating catalog URL. Downgrades and
different bytes under an already-installed version are rejected. A checksum
detects mismatched downloads; it is **not** a publisher signature. Reverse-domain
IDs are names, not proof of domain ownership. Trust comes from the catalog the
person chooses and its HTTPS delivery. Keep catalog URLs stable.

## Build and host a catalog

Put the latest package for each plugin into a source folder, then run:

```sh
cargo run -p sovra-skills --example publish_catalog -- \
  https://YOUR-ACCOUNT.github.io/YOUR-REPOSITORY \
  examples/plugins/packages /tmp/sovra-plugin-site
```

The output directory must be new. The builder uses the app's actual validation,
rejects duplicate identities, sorts entries by title, writes immutable
`packages/<sha256>.json` files, and generates linked pages of 48 entries. A build
accepts up to 10,000 input files. It reads one package at a time rather than
holding all instruction bodies in memory. Copy the generated output into the
catalog repository's publishing folder, preserving the `packages/` directory.
The example's placeholder addresses are replaced by the supplied base URL.

Configure that repository's **Settings → Pages** to publish its chosen branch
folder or a GitHub Actions artifact. GitHub supports branch publication from the
root or `/docs`; Actions can publish the generated artifact directly. See
[GitHub's publishing-source instructions](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site).
The app needs the final `<base-url>/catalog.json` address, not the repository URL.
No repository is created or published by the builder.

Publish packages before the catalog that references them, and keep old package
URLs available while clients may still have earlier catalog pages. Publish a new
version for every package change. A catalog deployment should replace its pages
together so page navigation does not mix releases.

## Scaling and boundaries

| Layer | What grows, and how it is bounded |
| --- | --- |
| Discovery | Paged metadata, no startup network crawl and no eager package downloads |
| Installed inventory | Up to 4096 entries and an 8 MiB private JSON index; the UI renders 20 installed entries at a time |
| Package storage | Separate immutable files named by checksum; updates replace the index atomically, not every installed package |
| Activation | Only enabled package contents enter the skill catalog; Sovra's existing 256-skill limit includes built-in and personal skills, and activation fails clearly if that budget is exceeded |
| Failure isolation | A damaged active package is reported and omitted at startup while installed controls remain available to turn it off |
| Updates | Explicit, serialized, checksum-verified and identity-bound; prior cache blobs are retained, with no cache collection or rollback UI yet |

Core owns validation, identity rules, commands/events and the UI reducer.
`sovra-skills` owns private storage and publisher filesystem IO behind the core
`PluginStore` port. It reuses the existing bounded file helpers rather than
adding another archive extractor. `sovra-runtime` owns bounded HTTP downloads and
one serialized library worker; shutdown cancels its task. `sovra-app` supplies the
plugin storage path once. The UI sends commands and renders events.

Version 1 establishes that boundary for future content kinds. Executable plugins,
declared permissions, dependency solving, signed catalogs, global remote search,
cache collection and multi-catalog discovery need explicit contracts and tests
before being supported. New kinds must get an API revision or a deliberately
compatible schema change; accepting arbitrary unknown fields is not an extension
mechanism.
