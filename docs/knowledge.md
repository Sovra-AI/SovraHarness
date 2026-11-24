# Reviewing what Sovra knows

Open **Knowledge** and use **Search your knowledge** to find wording across
files, saved personal details, and artifacts. Below search, **Files** shows
folder activity, **Relationships & categories** shows how things in your files
connect and what kind each file is, **About you** opens saved personal details, and **People** lists
contacts. **Guide** explains how Sovra remembers. Expand the sections you need
together. **Overview** folds them and returns to search. You can reopen the
last search results there.

## About you

**Remember across conversations** controls automatic learning and recall. Turn it
off to pause both; your saved history stays available. Messages used for learning
may be sent to your selected AI service, even though the resulting details are
stored on this computer.

Search saved details by a name, topic or project, then choose **Search / refresh**.
Open a detail to inspect its wording, dates and supporting record. An original
record can be saved even when no useful detail was extracted from it. Extraction
runs in the background, so a detail from a new message may appear only later.

Use **Read full original message**, **Read original record**, or **View linked
details** where offered to move between an interpretation and its evidence.
The **Saved details** citation card in chat opens the same Knowledge records.
A citation is a navigation aid, not proof that a reply is correct.

## Open the knowledge map

Choose **Knowledge → Open knowledge map**. The coloured collection buttons control
files, findings, relationships, personal records, saved work, people and visible
group spaces independently. The map opens without a search. Folder, category and
finding totals describe the index. The point and connection counts above the
drawing describe the current view. The graph fills the workspace. Open **Search and filters** (**Filters** in a
narrow window) for collection and display controls. **Back to Knowledge** restores
the regular workspace.

Select a point and choose **Draw connected records** to add its next page to the
same drawing. **Draw more across collections** adds a page from each enabled collection
and folder. **Group by connections** brings related points closer together.
**Find on map** steps through drawn points matching your words. Select a saved relationship to follow its subject, object and
cited passage. A **mentions topic** link means two records use the same label.
**Add search results** can bring matching excerpts and equivalent readings from
files, saved details and artifacts onto the map. Each source keeps its own identity.

Scroll up over the graph to zoom in at the pointer, or scroll down to zoom out.
Drag to pan. Pinch and the zoom buttons also work. **Fit map** restores the overview. Use **Neighbours only** to inspect one point and its direct links.
**Read as a list** gives keyboard access to the points and their connections.
**Include history** reopens the map with earlier records, file observations and
replaced relationships available. **Refresh** reads the local sources again.

The layout uses collection aggregates, indexed continuation pages, batched line
drawing and a limited number of non-overlapping labels. The design draws on
[research on large graph hierarchies](https://www.cs.ubc.ca/labs/imager/tr/2009/Archambault_TugGraph_PacificVis/)
and [Cytoscape’s rendering guidance](https://js.cytoscape.org/#performance).

## Explore your sources

The **Evidence map** shows how many excerpts and shared connections the search
returned. Choose an equivalent measurement or a linked identifier path, then
select a source to see its original line, date and findings. **All matching
sources** includes excerpts without a shared connection. **Source list** opens
the full set of result cards. Equal quantities can come from separate events.

Open **Knowledge** or use `/explore-knowledge`. Enter a term from your
sources and choose **Search sources**. For example, if a file says “Juniper weighs
0.5 kg”, your note says “My Juniper weighed 500 g”, and an artifact contains the
same reading, search for **Juniper**. The result names the sources that agree.
**Read source wording** shows the text used for each calculation. You can also
open the saved artifact, original personal record, or source folder.

Explore queries current folder grants, available private recall, and the current
versions of saved artifacts on this computer. Calculations run locally when you
search. A source that cannot answer reports its own status. The Files history
reader and About you record views remain the places to check earlier readings,
world-validity dates, corrections, and withdrawn evidence.

| Question | What the result uses |
| --- | --- |
| What kind of thing is this and how does it relate? | A table with `subject,predicate,object` gives ordered, named statements. Add `subject_type,object_type` to check a declared property. A shared namespaced ID can connect two sources. Simple English statements are labelled **possible connection from wording**. Sovra saves compatible source relationships during indexing. Wording in code, programs, archives and data files is not read as a fact; a relationship a table names outright still counts. |
| What shape do the samples form? | Two to sixteen numeric coordinate columns named `x`, `y`, `z`, or `dim1` through `dimN`, in a common scale. The first 64 points give connected-component and loop counts at stated Euclidean distances, including the longest loop’s birth and death when present. |
| What type of dataset fits? | Column names, finite numeric values, dates, units and missing cells provide the evidence. Optional named profiles keep ties visible. An unrecognized table is reported as unknown. |
| Do the measurements agree? | Decimal readings keep their original units and use compatible base units for comparison. Supply `value,unit,instrument,calibration_date,reference,uncertainty` for context. Add `reference_value,reference_unit,tolerance` for an accuracy check. Tolerance and uncertainty use the reading’s unit. Supported dimensions are mass, length, time, volume, temperature, pressure and ratio. |
| Which forms match? | Select English, French, German, Spanish, Italian, Portuguese, Dutch or Arabic under **Search options**. Word stems and observed changes explain matches. Folder identifiers such as `SITE-778` retain literal-token retrieval. Black-and-white image masks and plain `P1` bitmap text report foreground, erosion, dilation and inner-boundary pixel counts for a stated square and zero border. |
| What happened over time? | Dated table rows use event time, including out-of-order readings. Minute, hour and day windows give UTC counts and means per instrument and compatible unit. A date without a time means midnight UTC. |
| What does each result mean? | Source type, provenance, line numbers, measurement units, and wording are readable in text. Possible interpretations retain their label. A shared numeric value establishes agreement on the quantity alone. |

CSV, TSV and Markdown pipe tables can carry observations. Each analysis takes up
to 64 KiB and 1,024 rows. Connected CSV and TSV files that fit this budget use the
whole extracted table, even when a later row matched. Larger files expose the
matching passage and its line range. Mask image inputs are limited to one million
pixels (1,048,576), with opaque black as foreground and white or transparent as
background. SVG uses rendered pixels.

Under **Search options**, **Custom rules for experts** also accepts JSON. This example defines a table profile,
an hourly window, and a property whose object must be a grower.

```json
{
  "language": "english",
  "window_seconds": 3600,
  "mask_radius": 1,
  "profiles": [
    {"name": "Scale readings", "required_columns": ["unit", "value"], "numeric_columns": ["value"]}
  ],
  "properties": [
    {"id": "food:grown_by", "subject": {"domain": "product"}, "object": {"domain": "grower"}}
  ]
}
```

The graph uses stable identifiers, declared relationship ends, and a directed
category hierarchy. Topology uses a Vietoris–Rips filtration. Unit normalization
covers a subset of UCUM, and word matching uses language-specific stemmers.
[W3C RDF Schema](https://www.w3.org/TR/rdf-schema/) · [GUDHI Rips complexes](https://gudhi.inria.fr/python/latest/rips_complex_user.html) · [UCUM](https://ucum.org/ucum) · [Snowball algorithms](https://snowballstem.org/algorithms/) · [OpenCV morphology](https://docs.opencv.org/4.x/d9/d61/tutorial_py_morphological_ops.html)

## Explore the connections

**Found in your files** shows the live file and finding counts as Sovra analyzes
granted folders locally, and **See relationships** opens the section below.
Previously indexed folders catch up from their saved passages.

**Relationships & categories** reads from top to bottom:

| Part | What it shows |
| --- | --- |
| Numbers at a glance | Files sorted, connections, kinds of things, and other findings, then pills for what your files are, the parts of life they cover, and the things they mention. |
| Suggestions waiting for you | Relationships a helper proposed. Read the file's wording, then **Save these** or **Don't save**. Only shown when there are some. |
| Relationships | Each saved relationship as a short sentence, such as *Basil grown by Anna*, with the kinds of things it joins and how Sovra knows it. Search by a word, or choose **All** or a relationship type found in your files. Open one to see its facts, **Everything about** either side, and **Where it came from**, where **Read** opens the saved passage. Tick **Show changed files** or **Show replaced ones** to see older relationships. |
| More that Sovra noticed | Measurements, dates and times, tables, word forms and other findings, one kind at a time. **Source** shows the passage beside a finding and **Show more** pages through the rest. |
| What Sovra can recognise | Folded by default: every relationship type (choose one to filter), kinds of things, kinds of files and parts of life. A type a file defined for itself shows the kinds it joins, such as *grown by (Product → Grower)*. |

Under **About you**, the native map shows saved personal details and their connections. Select a label to
inspect its story, or filter the kinds of connections displayed. **Focus on these
connections** narrows the view to the selected detail and its direct connections;
**Show all** restores the overview. **Read as a list** provides a text alternative.
The map is a way to explore saved evidence, not a claim that every possible
relationship has been found.

## Correct or stop using information

Corrections keep the earlier version in history rather than overwriting it.
Stopping one detail excludes that detail from later recall, but separately
supported details can remain usable. To stop an entire original record and its
linked details, open it and choose **Stop using this record**, then
**Confirm stop using**. Pending checks for that source are cancelled.

These actions retain records for review; they are not permanent deletion or
secure erasure. Stopped records can still be inspected when included in the
review view. They cannot retract content already sent to a provider or copied
elsewhere. For the exact scope of fact, correction-family and source withdrawal,
see [the memory contract](memory-capture-and-recall.md#how-withdrawal-blocks-recall).

## People and files

**People** lists saved people and their contact details, with WhatsApp groups
shown separately. Check the selected recipient and message review before sending;
see [Messaging](communications.md). Sharing a first name alone is not enough to
merge two people.

**Files** provides access to folder knowledge. It is separate from private chat
memory and from files attached to one message. See [Files in chat](files.md) and
[Skills and file knowledge](skills-and-file-knowledge.md) for reading, indexing,
source access and model choices. **Guide** explains the memory concepts; examples
in that section are illustrative rather than your own saved facts.

Search your folders, then press **Save this answer** under a passage. The card
under **Answers you saved** names the file, the lines, the wording, and the day
that version was read. That day is the UTC day Sovra read the version. It is
not a date written in the file, and it is not the file's modified time. A later edit
of the file does not replace the saved wording. A new search shows the updated
text. Saved cards survive an app restart. Removing the folder also deletes its saved quotations. See
[Save the wording you read](skills-and-file-knowledge.md#save-the-wording-you-read).

Shared knowledge between computers is managed separately in
[Groups](groups.md). Local storage and backup limitations are covered
in [Configuration and local data](configuration.md).
