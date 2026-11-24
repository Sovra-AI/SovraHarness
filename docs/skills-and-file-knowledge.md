# Skills and folder knowledge

Use **Skills** to discover what Sovra can do and keep reusable instructions.
Use **Knowledge → Files** to grant access to folders and search their saved text.
Neither an imported skill nor a model's request grants access to new files or tools.

## Find and use a skill

1. Open **Skills** and search or browse the capability cards.
2. Choose **Try in chat** to add the skill to your draft as a pill with its name and icon.
   **Try the example** also fills in a sample request. Edit the request, then press **Send**
   to run the skill or open its workspace action.
3. You can also type `/` in the composer. Choose a shortcut with Tab or Enter, or type its
   full name and a space. Missing arguments produce a usage hint.

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
`sovra-example` in a skill's metadata is the everyday request its **Skills** page
card shows and **Try the example** puts in the chat.

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
cancelling a scan also pauses them. **Remove**, then **Remove index**, revokes
indexed retrieval and deletes that folder's index and saved quotations without
changing the original files. Sovra says the folder is being removed until it
leaves the list. Re-adding a folder does not restore old revoked access
references. A cancelled or incomplete scan does not mark all unvisited files
as deleted.

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
  History is not a byte-for-byte backup of the original file, and it is not
  where a saved answer keeps its wording.
- `/map-files` proposes relationships grounded in source passages. Review and
  explicitly save a proposal in **Knowledge → Relationships & categories** before it becomes reviewed knowledge. Model
  confidence is not a calibrated truth score. Source access, provenance, and
  revocation remain separate from accepting an interpretation.
- `/file-findings` (**See what's in your folders**) lets a helper use what Sovra
  already worked out on its own. `overview_file_knowledge` gives the counts:
  file kinds, parts of life, kinds of things mentioned, relationship types, and
  how many findings of each kind there are. `list_file_findings` lists one kind
  at a time — `measurement`, `dated_series`, `table`, `relationship`,
  `word_form`, `number_pattern` or `reading_limit` — and can keep only findings
  whose wording or file path contains some text. Each finding carries the path,
  line and a passage reference, and the helper reads that passage before giving
  an exact value. Every chat turn with connected folders tells the model these
  tools exist. Delegated helpers get them with the other read-only file tools;
  they never get `propose_file_knowledge`. When no passage matches a
  question's wording, the turn instead carries up to six findings whose
  wording or file path contains one of its first three distinctive words.
- File search requires every word of a short search. A search of three or
  more distinctive words that matches nothing is retried with any of them,
  ranked so passages with more of the words come first. Every timestamp in a
  file tool result also has a `<name>_utc` copy in plain UTC.
- Matching passages are inserted just before the latest question. For Ollama,
  OpenAI-compatible servers and OpenRouter, Sovra then sends one leading system
  message and turns any later system entry into a user-role note. Some servers
  keep only the first system message or reject later ones.
- A question that sounds current ("latest", "today") is forced to web search
  only when the connected folders have no matching passage, unless the person
  explicitly asks to search the web.
- `query_file_knowledge` returns every saved relationship. Each one's
  `basis.tier` says where it came from: `deterministic_extraction` (a table),
  `heuristic_extraction` (sentence wording, so check the cited passage), or
  `model_inference` (a helper's proposal that you approved).

Reviewed proposals can introduce a stable domain class or property identifier.
A property declares the subject and object classes it accepts. Follow a named
relationship in either direction or use **is a** to follow the category hierarchy.
The review retains the cited version and rejects incompatible endpoints and
category cycles. For local analysis across private notes, files, and saved work,
see [Explore your sources](knowledge.md#explore-your-sources).

Readers use opaque, version-bound references. Refreshing or revoking a source
invalidates stale current references instead of combining passages from different
versions. Partial text remains labelled even when every saved part has been read.

## Save the wording you read

On **Your folders**, search first, then press **Save this answer** under the
passage you want to keep. You do not need a shortcut to do this.

The saved card names three things:

- **File** — the file the passage came from.
- **Passage** — the lines and the wording Sovra read.
- **Read on** — the UTC day that version was read. This is not a date written
  in the file, and it is not the time the file was last modified.

The folder index keeps the quotation and its citation across restarts. Repeated saves of the same passage keep one card. Search again to
see the updated text. The card under **Answers you saved** still shows the
wording, file, lines, and read date from the version you saved.

Saving does not change the file. If you remove the folder, that saved answer
is deleted with its grant. `/file-history` still only inspects retained
observations.

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
