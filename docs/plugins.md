# Downloadable plugins

Sovra's first plugin API bundles reusable instructions and suggested service
connections. There are two ways to get one: paste a public GitHub repository
address, or load a catalog that a publisher hosts as static JSON files (for
example on GitHub Pages). Either way, Sovra stores the plugin on this computer
and checks it before use. Neither needs a running registry server.

## The Plugins page

Open **Plugins** in the menu on the left. **Your plugins** lists each plugin
as a card: where it came from, its version, an **On** or **Off** label, and its
buttons (**Turn on**/**Turn off**, **What's included**, **Check for update** for
GitHub plugins, and **Remove**). **What's included**, an update's list of
changes, and the remove warning open inside that plugin's card. A search box
appears once there are more than three plugins or a catalog is loaded.

With no plugins yet, the page offers **Add the example plugin**, which adds the
[plugin template](https://github.com/Sovra-AI/sovra-plugin-template) from GitHub
(switched off, like any other). **Add a plugin** has a card for each way in,
**From GitHub** and **From a catalog**, side by side when the page is wide
enough, and **Open the plugin template** for making your own. Messages about
the last action appear at the top of the page; problems are shown as warnings.

## Start from the template

The quickest way to make a plugin is the open-source (MIT)
[plugin template](https://github.com/Sovra-AI/sovra-plugin-template). Choose
**Use this template**, make the copy public, change the example skill, and paste
the copy's address into **Add from GitHub**. Its source lives in this repository
at [`examples/plugin-template`](../examples/plugin-template), where a test checks
that it is added with every skill and file.

The template's **Check plugin** workflow runs the read-only repository check on
each push. Run it yourself on a local checkout with:

```sh
cargo run -p sovra-skills --example check_repository -- \
  /path/to/repository OWNER/REPOSITORY
```

It applies the same rules and the same code as **Add from GitHub** to the folder
(everything except `.git`, as it would be once pushed). It fails when nothing
can be added, or when anything would be left out, and names each item.
It lists the skills and suggested connections that would be added.

## Add from GitHub

Open **Plugins** (in the menu on the left), paste a repository address into **Add from GitHub**,
and choose **Add from GitHub**. These forms all work:
`https://github.com/owner/repo`, `github.com/owner/repo.git` and `owner/repo`.
Paste the repository's main address. A link to a folder or file inside it
(`/tree/…`, `/blob/…`) is refused.

Sovra reads the newest commit on the repository's default branch and takes:

- **Skills:** every `SKILL.md` in the
  [Agent Skills format](skills-and-file-knowledge.md#add-reusable-instructions),
  in any folder. Folders that start with a dot are skipped, except `.claude`.
  So are `node_modules`, `target`, `vendor` and `__pycache__`. Each skill's
  `name` must match its folder, apart from a `SKILL.md` at the repository root.
  When one skill name appears in two places, the first usable path in sorted
  order is used. A skill that can't be used (a file over 64 KB, a name that
  doesn't match its folder, unreadable frontmatter, or a name longer than 32
  characters) is left out and named in the message. The rest are still added.
  An optional `sovra-example` in a skill's `metadata` is the everyday request
  its **Skills** page card shows (one line, at most 200 bytes); without it, the
  card names the skill instead of showing its command.
- **Suggested connections:** servers in `.mcp.json` at the root and in the
  `mcpServers` object of `.claude-plugin/plugin.json` that have an HTTPS `url`.
  At most eight are kept. Servers that start a program (`command`) and plain
  HTTP addresses are left out. **Set up connection** works as it does for
  catalog plugins.

Each skill also keeps the **text files in its folder**: reference documents,
templates, and scripts (kept as text only). Files inside a nested skill's
folder belong to that skill. Hidden files, `node_modules` and the like,
pictures and other non-text files, and files over 128 KB are left out. A skill
keeps at most 64 files, and a repository at most 1.5 MB of them; when there are
more, the smallest files are kept first, so every skill gets its short guides.
The rest are left out. The message gives the number left out for each skill. The skill's
instructions end with a list of its files, so the helper can read one when the
instructions mention it (`load_skill` with the skill name and the file's path
as `resource`). Files can only be read while the plugin is switched on.

`allowed-tools` does not grant tools, and nothing from the repository is run,
scripts included. The title is `owner/repo`. The description comes from `plugin.json`, otherwise
from the repository's GitHub description.

A repository is added **switched off**, like a catalog download, and is fixed
to the commit it was read at. The plugin's card shows the first seven
characters of that commit. **Check for update** reads the newest commit again.
If the commit is the same, Sovra says it is up to date and downloads nothing
else. Otherwise it lists each skill and connection that the newer commit adds,
changes or removes, and any it would leave out. **Update** installs exactly what was listed, keeping the
on/off choice; **Not now** keeps the installed copy. A repository that is
already added cannot be added a second time; Sovra says so before asking GitHub
for anything. Use **Check for update** instead.

Files are downloaded six at a time from one shared queue, so one slow file
does not hold up the rest. Adding `anthropics/skills` (18 skills, about 240
files) takes one to two seconds on a typical connection. A skill file is read
from the package of the one plugin its skill belongs to, never by opening every
switched-on plugin.

Limits:

- Public repositories only. Requests are not signed in, so GitHub allows about
  60 an hour from one computer. Adding a repository makes three of those
  requests and an up-to-date check makes two; file contents are downloaded
  from the commit's raw file addresses.
- Up to 32 skills per repository (catalog packages stay at 16), 64 KB per
  `SKILL.md`, and 4 MB for the whole plugin with its files (catalog packages
  stay at 512 KB and cannot include files). Every skill also counts toward
  Sovra's 256-skill total.
- Redirects are refused. A renamed or moved repository asks for its new
  address. A plugin stays tied to the repository address it was added from.
- A repository is identified as `io.github.<owner>.<repo>` (lowercased). That
  is a name, not proof of ownership. Trust comes from the repository you choose.

## Remove a plugin

Choose **Remove** on an installed plugin, then **Click again to remove**. A
plugin that is switched on is switched off and removed in the same step: its
skills leave the Skills page and new chats at once. A chat already using one of
its skills finishes with the copy it started with, as with **Turn off**.

Removing deletes the plugin's stored copy from this computer, so nothing of it
is left behind. Connections you set up from its suggestions are yours and stay
in **Connections**; stop them there. An update you checked but did not take is
dropped too. To use the plugin again, add it from GitHub or download it from its
catalog again; it comes back switched off, at the newest version.

## Use a catalog

Open **Plugins** (in the menu on the left), paste the full HTTPS address of `catalog.json`, and
choose **Load catalog**. No default publisher is configured. The last successfully
loaded catalog address is saved; leaving the address box empty loads that address.
Its plugins appear under **Available from this catalog**; use **Previous page**
and **Next page** to browse. The search box searches the current remote page
and all installed metadata; it is not a global remote search.

**Add** downloads a plugin switched off (**Update** when a newer version of
one you have is listed). **What's included** shows its skills
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
updates, dependency downloads, or executable plugin API in version 1. To uninstall
one, see [Remove a plugin](#remove-a-plugin).

## Publisher contract: version 1

The authoritative parser and types are
[`sovra-core/src/plugins.rs`](../crates/sovra-core/src/plugins.rs).
The [example package](../examples/plugins/packages/weekly-planning-1.0.0.json)
and [example catalog](../examples/plugins/catalog.json) are checked in tests.
Unknown fields, repeated JSON fields, unsupported format versions and invalid
contents are rejected. All strings are UTF-8.

### Validate while authoring

The [version 1 JSON Schema](../schemas/plugins-v1.schema.json) describes both
packages and catalog pages, with reusable definitions for their entries, skills
and services. Associate it with those JSON files in your editor's settings for
field completion and structural validation. Keep the association in editor
settings: `$schema` is not an accepted field inside a plugin package or catalog.

Before publishing a collection, run the read-only check:

```sh
cargo run -p sovra-skills --example check_catalog -- \
  https://YOUR-ACCOUNT.github.io/YOUR-REPOSITORY \
  examples/plugins/packages
```

Exit status zero means the source packages and proposed catalog pages passed
the same preparation used by the catalog builder. A failed package identifies
its source path. The check leaves the source folder unchanged and creates no
publication folder, so it can run in a publisher's CI before deployment.

The schema catches field names, types, required contributions and array bounds.
The shared Rust parser also checks UTF-8 byte budgets, URL policy, unique plugin
IDs and skill names, and numeric version bounds. JSON Schema string lengths
count characters; they cannot replace the byte limits. Exact downloaded bytes,
checksums and catalog/package agreement are checked during publication and
installation. Contract tests validate the schema against its declared standard
and exercise both validators against the examples and invalid packages. The schema uses
[JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12/json-schema-validation).

### Version and extension rules

| Version | What it identifies | When to change it |
| --- | --- | --- |
| Package `version` | One immutable release of a plugin ID | Any change to the published package bytes, including descriptions and instructions |
| `schema_version` | The fields and structure of a package or catalog | A new field, contribution shape, or incompatible format change |
| `api_version` | What Sovra must understand to use a package | A new host capability or changed interpretation of existing contributions |

Version 1 accepts the `skills` and `services` contributions described below.
A new contribution needs its schema, core validation, runtime behavior and
contract tests implemented together before a publisher emits it. Bump the
relevant versions and preserve fixtures for supported older versions; do not
extend version 1 by putting executable behavior in instruction text or adding
unknown fields. A catalog can display an entry for a newer API, but the current
host will leave its download unavailable. Publishers targeting older hosts
should retain a compatible catalog URL while introducing a newer contract.

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
| `version` | Immutable `major.minor.patch`, each an unsigned 32-bit integer without leading zeroes; prerelease and range syntax are not accepted. That gap is on the [roadmap](roadmap.md) |
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
| Updates | Explicit, serialized, checksum-verified and identity-bound; there is no rollback UI |
| Cache collection | After every install, update or removal is saved, package files no installed entry names are deleted. Only regular `<sha256>.json` files are candidates; the index, staging files and anything else in the folder are left alone. A failed deletion is logged and retried after the next change, never failing the command |

Core owns validation, identity rules, commands/events and the UI reducer.
`sovra-skills` owns private storage and publisher filesystem IO behind the core
`PluginStore` port. It reuses the existing bounded file helpers rather than
adding another archive extractor. `sovra-runtime` owns bounded HTTP downloads and
one serialized library worker; shutdown cancels its task. `sovra-app` supplies the
plugin storage path once. The UI sends commands and renders events.

Version 1 establishes that boundary for future content kinds. Executable plugins,
declared permissions, dependency solving, signed catalogs, global remote search
and multi-catalog discovery need explicit contracts and tests
before being supported. Follow the version and extension rules above when adding
them.
