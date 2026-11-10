# Terminal access

How Sovra runs commands on this computer, with Allow once / Always allow on a
chat card.

Sovra can run commands on the person's computer through one tool,
`run_terminal_command`. Commands outside the read-only set and saved allow rules
wait for an approval card in chat. Risky or opaque forms always ask, even when a
saved rule would otherwise match. Approved commands run with your OS permissions.

## How a command is decided

```
model call ──► sovra-adk TerminalTool ──► TerminalAccess (port, sovra-core)
                                              │
                         sovra-runtime TerminalRuntime::run
                                              │
            TerminalPolicy::evaluate (pure, sovra-core/src/terminal)
               │                         │
         Verdict::Run               Verdict::Ask ──► Event::Terminal(Call AwaitingApproval)
               │                         │            ◄── Command::Terminal(Decide)
               ▼                         ▼
        process::run (tokio, own process group, stdin closed)
               │  Event::Terminal(Output) every 80 ms
               ▼
        TerminalOutcome::report_for_model ──► tool result
```

The assistant must call this tool and do the work on the computer. It must not
paste a command in chat for you to copy or run, and it must not ask in chat
whether it may run a command or change files. The card is how you approve
(**Allow once** / **Always allow** / **Don't allow**); **Always allow** remembers
the suggested rule (the program plus subcommand, or the program alone for
file-taking tools such as `mkdir`).

1. `parse::analyse` splits the command line into simple commands at `&&`, `||`,
   `;`, `|`, `|&`, `&` and newlines, removing quotes. It strips pass-through
   wrappers (`timeout 30`, `nice`, `time`, `nohup`, `stdbuf`, `command`) and
   leading `NAME=value` assignments, so `NO_COLOR=1 timeout 30 npm test` is
   checked as `npm test`.
2. Anything it cannot read with certainty makes the whole command **opaque**:
   `$(…)`, backticks, `( … )`/`{ … }` groups, process substitution, here-docs,
   a variable or wildcard as the program, unbalanced quotes, a dangling `&&`,
   or more than 10,000 bytes. Opaque commands always ask and are never
   remembered.
3. Every simple command must be **read-only** (`ls`, `cat`, `grep`, `git status`,
   `git log`, …, with no redirect writing a file) or match one of the person's
   **"always allow" rules**. Otherwise the person is asked.
4. Some commands always ask and are never remembered, whatever the rules say:
   - **Launchers**: shells and `eval`, `sudo`, `env`, `xargs`, `ssh`, `awk`, `watch`,
     environment runners, and interpreters running inline code such as `python -c`.
   - **Destructive programs**: `rm`, `dd`, `chmod`, `kill`, `shutdown`, `reg` and
     similar.
   - **Code-running forms of otherwise safe tools**: `find -exec` or `-delete`,
     `git -c …`, `git push --force`, `git reset --hard`, `rg --pre`, and `sed`
     (whose script can run programs).
   - **Background jobs** (`&`), which would outlive the time limit and the Stop button.
5. The card offers **Allow once**, **Always allow “npm test”** (when a safe rule
   fits) and **Don't allow**. The rule is the program plus its subcommand
   (`npm test`, `cargo build`, `git log`) or, for file-taking programs, the
   program alone (`mkdir`). For a script run by an interpreter it is the script
   (`python3 manage.py`) or module (`python -m pytest`). A compound command saves
   one rule per unapproved part, at most five.

These rules follow Claude Code's Bash permission model: compound-command
splitting, wrapper stripping, the read-only set, the exec-wrapper and `find`
exceptions, and a one-time-only prompt when the command can't be fully read.
Codex CLI's "untrusted" approval policy works the same way: trusted read-only
commands run, everything else asks. Text rules are a convenience, not a sandbox.
`/bin/rm` or `bash -c 'rm …'` are different text, so they ask rather than
match. An approved command runs with the person's own rights.

## Running a command

- **Shell.** On Windows: `pwsh`, else Windows PowerShell, with `-NoProfile
  -NonInteractive`, UTF-8 output and progress bars off. Elsewhere: the person's
  login shell if it is bash or zsh, else bash, else `/bin/sh`, with `-c`.
- **Environment.** Apps started from a desktop launcher miss `PATH` entries
  added in shell profiles (Homebrew, nvm, cargo). Sovra reads the login shell's
  environment once (`$SHELL -l -i -c 'env -0'`, 10 s limit, after a marker so
  profile noise is ignored), as editors do, and adds it to every command.
  `NO_COLOR`, `TERM=dumb`, `PAGER=cat`, `GIT_TERMINAL_PROMPT=0` and similar
  keep output plain and stop tools from waiting for input.
- **Isolation from hangs.** stdin is closed, so a program asking a question
  fails at once instead of hanging. Each command runs in its own process group
  (Unix) or new process group without a console window (Windows). On Unix the
  PTY used for output is not a controlling terminal, so a program that waits
  for a keyboard still sees EOF on stdin.
- **Stopping.** A timeout (default 120 s, at most 600 s), the Stop button,
  switching terminal access off, or an interrupted conversation *after the
  command has started* sends SIGTERM to the whole group, then SIGKILL after two
  seconds (`taskkill /T /F` on Windows). Dropping the tool future (the
  model-response timeout, or Stop on the chat) kills a **running** process the
  same way. A card that is still waiting stays decidable: a 60 s model-response
  timeout does not expire it, and **Allow once** / **Always allow** still start
  the process. The approval wait itself is ten minutes. If the shell exits while
  a background child still holds its output, Sovra waits 300 ms and then ends the
  group.
- **Output.** On Unix, stdout and stderr are a terminal (PTY) so programs that
  line-buffer on a tty flush as they print; the card can read both streams
  while the command is still running. On Windows they are pipes (same live-card
  contract). Bytes are decoded incrementally: lossy UTF-8 across reads, ANSI
  escape codes removed, carriage-return progress bars collapsed to their final
  state, and lines capped at 4 KiB. The chat receives batches every 80 ms (at
  most 500 lines per batch, extra lines counted) and keeps the last 500 lines per
  card. The model receives the first 12 KiB and last 18 KiB with a note of what
  was left out.
- **Time limits.** ADK cancels tools after five minutes. When the terminal tool is
  offered, the orchestrator's tool timeout is raised to cover the ten-minute
  approval wait plus the longest command.

## The host description

At start-up the runtime detects the operating system and version
(`/etc/os-release`, `sw_vers`, `ver`), architecture, shell and starting folder.
These go into the tool description, so the model writes PowerShell on Windows,
BSD flags on macOS and so on.

## Settings and storage

`TerminalPolicy { enabled, rules }` is saved as versioned JSON at
`<app data>/terminal/settings.json` with write-then-rename. Settings → Terminal
switches access on or off and lists the always-allowed rules, each with
**Remove**. Switching off declines waiting cards, stops running commands and
removes the tool from the next turn. The terminal is offered only in private
conversations, never to WhatsApp or Telegram senders.

## Where the code is

| Concern | File |
| ------- | ---- |
| Parsing, classification, policy, protocol, state, labels | `crates/sovra-core/src/terminal/` |
| Tool definition and timeout | `crates/sovra-adk/src/terminal.rs` |
| Approval wait, persistence, processes, host detection | `crates/sovra-runtime/src/terminal*` |
| Chat card and Settings section | `crates/sovra-ui/src/terminal_card.rs`, `terminal_settings.rs` |
| Composition (paths, `$SHELL`, `PATH`) | `crates/sovra-app/src/main.rs` |
