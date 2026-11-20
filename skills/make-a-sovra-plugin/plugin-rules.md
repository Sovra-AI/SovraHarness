# Sovra plugin rules (Add from GitHub)

The full guide is `docs/plugins.md` in SovraHarness. These are the rules a plugin
repository must meet so nothing is left out.

## Layout

```text
.claude-plugin/plugin.json   one-line "description" of the plugin (optional)
.mcp.json                    suggested online connections (optional)
skills/
  <skill-name>/
    SKILL.md                 required for each skill
    any-guide.md             optional text files the skill can read
```

A `SKILL.md` can live in any folder. Folders starting with a dot are skipped,
except `.claude`. So are `node_modules`, `target`, `vendor` and `__pycache__`.
If two folders share a skill name, the first in sorted path order is used.

## SKILL.md

```markdown
---
name: birthday-message
description: Use when someone wants a short, warm birthday message for a friend or relative.
metadata:
  sovra-title: Write a birthday message
  sovra-example: Help me write a birthday message for my grandson
---

1. Ask who it is for and how old they are turning, one question at a time.
2. Write three warm sentences.
```

| Part | Rule |
| --- | --- |
| `name` | Same as the folder name. Lowercase letters and digits joined by single hyphens, at most 32 characters. |
| `description` | When to use the skill. Sovra reads it to pick the skill on its own. |
| `sovra-title` | Optional friendly title for the Skills page. |
| `sovra-example` | Optional everyday request. One line, at most 200 bytes. |
| Size | Each `SKILL.md` at most 64 KB. |

`allowed-tools` gives no tools. Nothing in the repository is run; scripts are
kept as text only.

## Limits

- Public repositories only.
- Up to 32 skills per repository; every skill counts toward Sovra's 256-skill total.
- Text files beside a skill: up to 64 per skill, 128 KB each, 1.5 MB per repository.
  Pictures and other non-text files are left out.
- Whole plugin with its files: at most 4 MB.
- Connections: up to eight servers with an `https://` `url`. Servers that start
  a program (`command`) and plain `http://` addresses are left out.

```json
{
  "mcpServers": {
    "My notes service": { "url": "https://notes.example.com/mcp" }
  }
}
```
