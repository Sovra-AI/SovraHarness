# Sovra plugin template

Start your own [Sovra](https://github.com/Sovra-AI/SovraHarness) plugin. A plugin
gives Sovra new skills: written instructions that tell a helper how to do a job,
like writing a thank-you note. This template already works. Copy it, change the
words, and add it to Sovra.

This template is open source under the [MIT License](LICENSE). Your copy is yours:
keep that license or choose another one.

## 1. Make your copy

1. Choose **Use this template** → **Create a new repository** at the top of this page.
2. Give it a name, for example `my-writing-helpers`.
3. Choose **Public**. Sovra can only add public repositories.

## 2. Add it to Sovra

1. In Sovra, open **Plugins** in the menu on the left.
2. Paste your repository's address, for example
   `https://github.com/your-name/my-writing-helpers`, into **Add from GitHub**.
3. Choose **Add from GitHub**. The plugin is added switched **off**.
4. Choose **What's included** to see its skills, then **Turn on**.

Now ask Sovra something like *"Help me thank my neighbour for watering my plants"*
and it uses the thank-you skill. Your skill also appears on the **Skills** page
under **From plugins**.

When you change your repository later, choose **Check for update** in Sovra. It
shows what changed, and nothing changes until you choose **Update**.

## 3. Write your own skills

Each skill is a folder inside `skills/` with a `SKILL.md` file:

```text
skills/
  thank-you-note/
    SKILL.md        the instructions (required)
    tone-guide.md   extra text the helper can read (optional)
```

`SKILL.md` starts with a short header between `---` lines, then the instructions:

```markdown
---
name: thank-you-note
description: Use when someone wants to thank a person for a gift or a favour.
metadata:
  sovra-title: Write a thank-you note
  sovra-example: Help me thank my neighbour for watering my plants
---

Ask who the note is for, then write three to five warm sentences.
```

| Part | What to write |
| --- | --- |
| `name` | The folder's name, exactly. Lowercase letters and numbers joined by single hyphens, at most 32 characters, like `thank-you-note`. |
| `description` | When Sovra should use this skill, in one or two sentences. Sovra reads it to pick the right skill on its own. |
| `sovra-title` | Optional. The friendly name people see. Without it, the name is used with spaces. |
| `sovra-example` | Optional. Something a person might ask, in their own words. The **Skills** page shows it, and **Try the example** puts it in the chat. One line, at most 200 characters. |
| The instructions | Plain steps, as you would explain the job to a helpful person. |

To add a skill, copy the `thank-you-note` folder, rename the folder, and set
`name` to the new folder name. Delete the example when you don't need it.

Put the plugin's one-line description in `.claude-plugin/plugin.json`. Sovra shows it
under the plugin's name. The same layout also works as a Claude Code plugin.

### Limits

Sovra leaves out anything it can't use and tells you what it left out:

- Up to 32 skills. Each `SKILL.md` can be up to 64 KB.
- Text files next to a `SKILL.md` (guides, templates, examples) come with the
  skill: up to 64 files per skill, 128 KB each, and 1.5 MB in total. Pictures and
  other non-text files are left out.
- Folders that start with a dot (except `.claude`), and `node_modules`, `target`,
  `vendor` and `__pycache__`, are skipped.
- Nothing in the repository is run. Scripts are kept as text only, and
  `allowed-tools` does not give a skill any tools.

## Suggest a connection (optional)

If your skills work best with an online service that speaks MCP, add a
`.mcp.json` file at the top of the repository:

```json
{
  "mcpServers": {
    "My notes service": { "url": "https://notes.example.com/mcp" }
  }
}
```

Sovra shows it under **What's included** with a **Set up connection** button. The
person still decides whether to connect. Only `https://` addresses are suggested;
servers that start a program on the computer are left out. Only add this for a
service you trust and that really exists.

## Check before you share

Every push runs the **Check plugin** workflow (the **Actions** tab). It runs
Sovra's own "Add from GitHub" rules on your repository and fails if anything
would be left out, so a green check means Sovra adds every skill.

To run the same check on your computer, with [Rust](https://rustup.rs) installed:

```sh
git clone https://github.com/Sovra-AI/SovraHarness
cd SovraHarness
cargo run -p sovra-skills --example check_repository -- /path/to/your/repository your-name/my-writing-helpers
```

More detail: [Sovra's plugin guide](https://github.com/Sovra-AI/SovraHarness/blob/main/docs/plugins.md).
