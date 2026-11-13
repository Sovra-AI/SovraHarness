# Skills and folder knowledge

Use **Skills** to discover what Sovra can do and keep reusable instructions.
Use **Knowledge → Files** to grant access to folders and search their saved text.
Neither an imported skill nor a model's request grants access to new files or tools.

## Find and use a skill

1. Open **Skills** and search or browse the capability cards.
2. Choose **Open now** for a workspace action, or **Try in chat** / **Try the example**
   to prepare a draft. Review the draft before sending.
3. You can also type `/` in the composer to search shortcuts and choose one with
   the keyboard. Missing arguments produce a usage hint.

An explicit shortcut selects a procedure before model execution. Some actions,
such as opening a page, do not need a model call. Model procedures keep your
request separate from their instruction text.

## Add reusable instructions

1. Choose **Add your own** on Skills.
2. Enter a title, description, and instructions. The title can use your language.
   Leave **Shortcut** blank to generate an unused one.
3. Choose whether to enable **Use it on its own when a request fits**.
4. Save and wait for confirmation. Use **Edit** on the saved skill for later changes.

The editor retains your draft if saving fails. Back asks before discarding
unsaved changes. Typing during a save preserves the newer draft and reminds you
to save again. Further saves update the same skill after its first confirmed save.

**Import** accepts a local skill folder with `SKILL.md`. The package is copied to
managed storage and validated; importing does not run scripts. Instructions and
bounded resources are loaded on demand. Packages have limits on files, bytes,
resource size, and paths. See [the package adapter](../crates/sovra-skills/src/lib.rs)
for the exact validation. Keep credentials out of skill files.

When automatic use is enabled, the model receives a bounded list of applicable
skill descriptions and may call `load_skill`. Installed procedures have priority;
tool availability constrains the list. A model-selected skill is not treated as
an explicit user selection. Untrusted messaging senders do not receive private
skills or their resources. Package metadata can disable automatic use with
`sovra-auto-apply: false` or `disable-model-invocation: true`.

## Add and read folders

1. Open **Knowledge → Files** and add folders, or use `/index-folder` in chat.
2. Select one or more folders in the native picker. Cancelling keeps your chat draft.
3. Wait for scan progress and review the **Scan report** for unreadable, skipped,
   or partially read files. Registration alone does not mean a folder is fully read.
4. Search the indexed text or ask a question in chat. Open citations to check the
   saved passage against its source.

The index stores snapshots of extracted text and file metadata, not a backup of
all original bytes. Ordinary chat and eligible helpers can retrieve relevant
passages without a filename in the question. The query's folder grants still
bound access. External models may receive retrieved passages when answering.

Use **Refresh** to reconcile changes. **Pause** stops automatic updates;
cancelling a scan also pauses them. Removing a folder revokes indexed retrieval
and deletes its index without changing the original files. Re-adding a folder
does not restore old revoked access references. A cancelled or incomplete scan
does not mark all unvisited files as deleted.

Folder settings control document-reading budgets and optional meaning search.
For supported formats, partial-reading warnings, saved-text navigation, and
resource limits, see [Files](files.md#folder-indexing-versus-attachments).
For progress and classification, see [File understanding](file-ontology.md).

## Search, history, and reviewed relationships

- `/search-files` finds relevant saved passages; `/file-catalog` lists observed
  metadata with filters. Filename and report filters use literal text, not SQL,
  regular expressions, or shell globs.
- `/file-history` inspects retained observations and extracted-text versions.
  Filesystem timestamps are not dates of events described inside a document.
  Historical reads require the original folder grant to remain valid.
- `/map-files` proposes relationships grounded in source passages. Review and
  explicitly save a proposal before it becomes reviewed knowledge. Model
  confidence is not a calibrated truth score. Source access, provenance, and
  revocation remain separate from accepting an interpretation.

Readers use opaque, version-bound references. Refreshing or revoking a source
invalidates stale current references instead of combining passages from different
versions. Partial text remains labelled even when every saved part has been read.

## Meaning search

Meaning search is optional per folder. Choose an embedding model explicitly;
adding a folder or selecting a chat model does not authorize bulk embedding.
The implemented transports are Ollama `/api/embed` and compatible `/embeddings`.
Approval is bound to the model and connection destination. Editing that destination
requires choosing again. Requests can send file text to that endpoint and incur costs.

Lexical search remains available independently. Vector search checks current
versions and grants, scans eligible vectors within its bounded deadline, and does
not establish large-corpus semantic quality. See [Retrieval benchmarks](knowledge-benchmark.md).

## Developer reference

| Boundary | Source |
| --- | --- |
| Skill catalog, shortcuts, validation, automatic-use guide | [core skills](../crates/sovra-core/src/skills.rs) |
| Built-in capability registration | [catalog](../crates/sovra-core/src/skills/builtin.rs) |
| Package IO and resources | [sovra-skills](../crates/sovra-skills/src/lib.rs) |
| Skill tools and dispatch | [ADK](../crates/sovra-adk/src/skills.rs), [runtime](../crates/sovra-runtime/src/skills.rs) |
| File versions, grants, queries, and reviewed knowledge | [core file index](../crates/sovra-core/src/file_index) |
| SQLite storage and readers | [file index adapter](../crates/sovra-files/src/index) |

The code also contains summary storage, clustering, and provider adapters.
These are library foundations: there is no integrated automatic summary scheduler
or desktop summary-generation workflow. They must not be presented as a working
user feature. File changes through terminal tools use the separate
[terminal approval flow](terminal.md); indexing itself does not edit source files.
