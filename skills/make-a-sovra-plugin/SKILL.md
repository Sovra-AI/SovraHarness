---
name: make-a-sovra-plugin
description: Use when someone wants to make their own Sovra plugin, write a skill for it, share it on GitHub, or find out why Sovra left part of their plugin out.
metadata:
  sovra-title: Make your own Sovra plugin
  sovra-example: Help me make a Sovra plugin that writes birthday messages
---

# Make your own Sovra plugin

Help the person turn something they want Sovra to be good at into a plugin they
can add from GitHub. Use plain words: say "skill" and "plugin", not "manifest" or
"frontmatter".

1. Ask what job the plugin should help with, if they have not said. One plugin can
   hold several skills; suggest one skill per job.
2. Tell them to start from the template: open
   <https://github.com/Sovra-AI/sovra-plugin-template>, choose **Use this template**,
   and make the copy **Public** (Sovra can only add public repositories). Its
   `thank-you-note` skill is a working example to copy and rename.
3. For each skill, write a complete `skills/<name>/SKILL.md` for them, following
   `plugin-rules.md`:
   - `name` is the folder name exactly.
   - `description` says *when* to use the skill, in one or two sentences.
   - `metadata.sovra-title` is the friendly card title.
   - `metadata.sovra-example` is something a person would really ask.
   - The instructions are short numbered steps, written as you would explain the
     job to a helpful person.
4. If the skill needs longer guidance (a style guide, a template letter, a
   checklist), put it in a text file next to `SKILL.md` and mention it by file name
   in the instructions. Sovra reads it when it is needed.
5. Put a one-line description of the whole plugin in `.claude-plugin/plugin.json`.
6. Explain how to add it: in Sovra open **Plugins**, paste the repository address
   into **Add from GitHub**, then **What's included** and **Turn on**. After
   later changes, **Check for update**.
7. Offer the check: the template's **Check plugin** workflow runs on every push,
   or locally with
   `cargo run -p sovra-skills --example check_repository -- <folder> <owner/repo>`
   from a SovraHarness checkout. A green check means nothing is left out.

If Sovra said it left something out, compare the named skill with
`plugin-rules.md` and fix the first rule it breaks.

Never suggest putting passwords, keys or private details in a plugin: plugins
are public. Only suggest a connection (`.mcp.json`) for an `https://` service the
person trusts and that really exists.
