//! What a simple command does: read-only, able to run other code, risky, and
//! which "always allow" rule fits it.
//!
//! The lists are deliberately short and conservative. A command missing from
//! them simply asks; a command wrongly added would run unasked, so additions
//! need a reason.

use serde::{Deserialize, Serialize};

use super::parse::{Redirect, SimpleCommand, Word};

/// Plain-English warnings shown on the approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Risk {
    /// Removes files or folders.
    DeletesFiles,
    /// Asks for administrator rights.
    NeedsAdmin,
    /// Downloads something and runs it straight away.
    RunsDownloadedCode,
    /// Installs or removes software.
    InstallsSoftware,
    /// Changes computer settings, permissions or running programs.
    ChangesSystem,
    /// Runs another program, script or code passed to it.
    RunsOtherCode,
    /// Talks to the internet or another computer.
    UsesNetwork,
    /// Creates or changes files.
    ChangesFiles,
    /// Keeps running after the command returns.
    RunsInBackground,
    /// Sovra could not fully read the command.
    HardToCheck,
}

impl Risk {
    /// Short label for the approval card.
    pub fn label(self) -> &'static str {
        match self {
            Self::DeletesFiles => "Deletes files",
            Self::NeedsAdmin => "Needs administrator rights",
            Self::RunsDownloadedCode => "Runs something it downloads",
            Self::InstallsSoftware => "Installs or removes software",
            Self::ChangesSystem => "Changes computer settings",
            Self::RunsOtherCode => "Runs other programs or code",
            Self::UsesNetwork => "Uses the internet",
            Self::ChangesFiles => "Creates or changes files",
            Self::RunsInBackground => "Keeps running in the background",
            Self::HardToCheck => "Sovra can't fully check this",
        }
    }

    /// Whether the card should colour this as a danger rather than a note.
    pub fn is_serious(self) -> bool {
        matches!(
            self,
            Self::DeletesFiles
                | Self::NeedsAdmin
                | Self::RunsDownloadedCode
                | Self::ChangesSystem
                | Self::HardToCheck
        )
    }
}

fn arg_texts(command: &SimpleCommand) -> impl Iterator<Item = &str> {
    command.args().iter().map(|word| word.text.as_str())
}

fn any_arg(command: &SimpleCommand, flags: &[&str]) -> bool {
    flags.iter().any(|flag| command.has_arg(flag))
}

/// Single-dash short-flag bundles such as `-rf` contain `letter`.
fn has_short_flag(command: &SimpleCommand, letter: char) -> bool {
    arg_texts(command).any(|arg| {
        arg.strip_prefix('-')
            .is_some_and(|rest| !rest.starts_with('-') && rest.contains(letter))
    })
}

fn subcommand(command: &SimpleCommand) -> Option<&str> {
    command.first_operand().map(|word| word.text.as_str())
}

/// Shells, interpreters and launchers whose arguments are themselves a command.
const LAUNCHERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "csh",
    "tcsh",
    "pwsh",
    "powershell",
    "cmd",
    "eval",
    "exec",
    "source",
    ".",
    "sudo",
    "doas",
    "su",
    "runas",
    "env",
    "xargs",
    "watch",
    "setsid",
    "ionice",
    "flock",
    "chroot",
    "unshare",
    "nsenter",
    "ssh",
    "parallel",
    "strace",
    "ltrace",
    "gdb",
    "lldb",
    "script",
    "expect",
    "at",
    "batch",
    "crontab",
    "systemd-run",
    "osascript",
    "start",
    "invoke-expression",
    "iex",
    "start-process",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "busybox",
    "nix-shell",
    "direnv",
    "devbox",
    "mise",
    "asdf",
];

/// Language runtimes: running a named script can be remembered, inline code cannot.
const INTERPRETERS: &[&str] = &[
    "python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php", "lua", "rscript",
    "julia", "tclsh",
];

/// Programs that delete or are hard to undo: always asked, never remembered.
const DESTRUCTIVE: &[&str] = &[
    "rm",
    "rmdir",
    "shred",
    "unlink",
    "dd",
    "mkfs",
    "fdisk",
    "sfdisk",
    "parted",
    "wipefs",
    "diskutil",
    "format",
    "del",
    "erase",
    "rd",
    "remove-item",
    "ri",
    "kill",
    "killall",
    "pkill",
    "taskkill",
    "stop-process",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "restart-computer",
    "stop-computer",
    "chmod",
    "chown",
    "chgrp",
    "chattr",
    "icacls",
    "takeown",
    "reg",
    "bcdedit",
    "truncate",
];

/// Whether the command can run arbitrary other code, or is destructive enough that no
/// saved rule or read-only shortcut may approve it.
pub fn always_asks(command: &SimpleCommand) -> bool {
    let name = command.program_name();
    let name = name.as_str();
    if LAUNCHERS.contains(&name) || DESTRUCTIVE.contains(&name) || name.starts_with("mkfs.") {
        return true;
    }
    if INTERPRETERS.contains(&name) {
        return interpreter_rule(command).is_none();
    }
    match name {
        "find" => any_arg(
            command,
            &[
                "-exec",
                "-execdir",
                "-ok",
                "-okdir",
                "-delete",
                "-fprint",
                "-fprint0",
                "-fprintf",
                "-fls",
                "-files0-from",
            ],
        ),
        // Global options before the subcommand (`-c`, `-C`, `--git-dir`) can make git run
        // configured programs or act on another repository.
        "git" => {
            command
                .args()
                .first()
                .is_some_and(|word| word.text.starts_with('-'))
                || git_is_destructive(command)
        }
        "rg" => any_arg(command, &["--pre"]),
        "sort" => any_arg(command, &["--compress-program"]),
        "tar" => any_arg(
            command,
            &[
                "--to-command",
                "--use-compress-program",
                "--checkpoint-action",
                "-I",
            ],
        ),
        // sed's `e` command runs programs and `w` writes files from inside the script
        // text, which a prefix rule cannot see.
        "sed" | "gsed" => true,
        "curl" | "wget" => any_arg(command, &["-K", "--config"]),
        _ => false,
    }
}

fn git_is_destructive(command: &SimpleCommand) -> bool {
    match subcommand(command) {
        Some("push") => {
            has_short_flag(command, 'f') || any_arg(command, &["--force", "--mirror", "--delete"])
        }
        Some("reset") => command.has_arg("--hard"),
        Some("clean") => has_short_flag(command, 'f') || command.has_arg("--force"),
        Some("branch") => has_short_flag(command, 'D'),
        _ => false,
    }
}

/// The rule for an interpreter running a script or module (`python3 manage.py`,
/// `python -m pytest`); `None` for inline code (`-c`, `-e`, `--eval`) or a bare REPL.
fn interpreter_rule(command: &SimpleCommand) -> Option<Vec<String>> {
    let program = command.program().to_owned();
    let mut args = command.args().iter();
    match args.next() {
        Some(word) if word.text == "-m" => {
            let module = args.next().filter(|word| plain_word(word))?;
            Some(vec![program, word.text.clone(), module.text.clone()])
        }
        Some(word) if !word.text.starts_with('-') && !word.dynamic && !word.glob => {
            Some(vec![program, word.text.clone()])
        }
        _ => None,
    }
}

/// A subcommand-like word: `test`, `run`, `build:prod`, `fmt`.
fn plain_word(word: &Word) -> bool {
    !word.dynamic
        && !word.glob
        && word.text.len() <= 40
        && word
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && word
            .text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        && !word.text.contains("..")
}

/// The words of the "always allow" rule that fits this command, or `None` when it
/// must be asked every time.
pub fn suggested_rule(command: &SimpleCommand) -> Option<Vec<String>> {
    if always_asks(command) || command.program().is_empty() {
        return None;
    }
    let name = command.program_name();
    if INTERPRETERS.contains(&name.as_str()) {
        return interpreter_rule(command);
    }
    let mut words = vec![command.program().to_owned()];
    if let Some(next) = command.args().first().filter(|word| plain_word(word)) {
        // `npm test`, `cargo build`, `git log`: the subcommand decides what runs.
        // For file-taking programs (`cat notes.txt`) the operand is just data, so
        // remember the program alone.
        if takes_subcommand(&name) || !looks_like_path(&next.text) {
            words.push(next.text.clone());
        }
    }
    Some(words)
}

fn looks_like_path(text: &str) -> bool {
    text.contains(['/', '\\', '.'])
}

/// Programs whose first word is a subcommand even when it contains a dot.
fn takes_subcommand(name: &str) -> bool {
    matches!(
        name,
        "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "cargo"
            | "git"
            | "go"
            | "dotnet"
            | "make"
            | "just"
            | "docker"
            | "kubectl"
            | "gradle"
            | "mvn"
            | "rake"
            | "uv"
            | "poetry"
            | "pip"
            | "pip3"
            | "brew"
            | "apt"
            | "winget"
            | "npx"
            | "pnpx"
            | "bunx"
            | "uvx"
    )
}

/// Programs that only read: they run without asking when nothing else in the
/// command needs approval.
const READ_ONLY: &[&str] = &[
    "ls",
    "dir",
    "pwd",
    "echo",
    "printf",
    "cat",
    "head",
    "tail",
    "wc",
    "which",
    "where",
    "whoami",
    "hostname",
    "uname",
    "date",
    "id",
    "df",
    "du",
    "stat",
    "tree",
    "diff",
    "cmp",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "true",
    "false",
    "test",
    "type",
    "cd",
    "uniq",
    "cut",
    "tr",
    "nl",
    "column",
    "less",
    "more",
    "sleep",
    "get-childitem",
    "gci",
    "get-content",
    "gc",
    "get-location",
    "gl",
    "select-string",
    "sls",
    "get-command",
    "gcm",
    "test-path",
    "resolve-path",
    "measure-object",
    "get-date",
    "get-item",
    "gi",
];

/// Whether this simple command only reads, so it can run without asking.
pub fn is_read_only(command: &SimpleCommand) -> bool {
    if always_asks(command) || command.writes_by_redirect() || !command.assignments.is_empty() {
        return false;
    }
    let name = command.program_name();
    let name = name.as_str();
    // A typed path (`./ls`) could be any program wearing a familiar name.
    if command.program().contains(['/', '\\']) {
        return false;
    }
    if READ_ONLY.contains(&name) {
        // These never finish when following a file, so they must be approved.
        return match name {
            "tail" | "less" | "more" => !any_arg(command, &["-f", "-F", "--follow"]),
            _ => true,
        };
    }
    match name {
        "find" => !command
            .args()
            .iter()
            .any(|word| word.glob && !word.text.starts_with('-')),
        "sort" => !any_arg(command, &["-o", "--output"]),
        "file" => !any_arg(command, &["-m", "--magic-file", "-f", "--files-from"]),
        "git" => git_is_read_only(command),
        _ => false,
    }
}

fn git_is_read_only(command: &SimpleCommand) -> bool {
    if any_arg(command, &["--output", "-o", "--ext-diff"]) {
        return false;
    }
    match subcommand(command) {
        Some(
            "status" | "log" | "diff" | "show" | "rev-parse" | "ls-files" | "blame" | "describe"
            | "shortlog" | "whatchanged" | "reflog" | "cat-file" | "grep",
        ) => true,
        // Listing only: `git branch -d x` / `git remote add` change things.
        Some("branch") => command.args().iter().skip(1).all(|word| {
            matches!(
                word.text.as_str(),
                "-a" | "-r" | "-v" | "-vv" | "--list" | "--all" | "--show-current"
            )
        }),
        Some("remote") => command
            .args()
            .iter()
            .skip(1)
            .all(|word| matches!(word.text.as_str(), "-v" | "--verbose")),
        _ => false,
    }
}

/// The plain-English warnings for one simple command.
pub(crate) fn risks(command: &SimpleCommand, out: &mut Vec<Risk>) {
    let name = command.program_name();
    let name = name.as_str();
    let mut add = |risk: Risk| {
        if !out.contains(&risk) {
            out.push(risk);
        }
    };
    if command.writes_by_redirect() {
        add(Risk::ChangesFiles);
    }
    if command
        .redirects
        .iter()
        .any(|redirect| matches!(redirect, Redirect::Write(target) if target.starts_with('/') || target.starts_with('~')))
    {
        add(Risk::ChangesSystem);
    }
    match name {
        "rm" | "rmdir" | "shred" | "unlink" | "del" | "erase" | "rd" | "remove-item" | "ri"
        | "truncate" => add(Risk::DeletesFiles),
        "dd" | "fdisk" | "sfdisk" | "parted" | "wipefs" | "diskutil" | "format" => {
            add(Risk::DeletesFiles);
            add(Risk::ChangesSystem);
        }
        "sudo" | "doas" | "su" | "runas" => add(Risk::NeedsAdmin),
        "kill"
        | "killall"
        | "pkill"
        | "taskkill"
        | "stop-process"
        | "shutdown"
        | "reboot"
        | "halt"
        | "poweroff"
        | "restart-computer"
        | "stop-computer"
        | "chmod"
        | "chown"
        | "chgrp"
        | "chattr"
        | "icacls"
        | "takeown"
        | "reg"
        | "bcdedit"
        | "systemctl"
        | "launchctl"
        | "sc"
        | "defaults"
        | "setx"
        | "crontab"
        | "set-executionpolicy" => {
            add(Risk::ChangesSystem);
        }
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "rsync" | "ftp" | "nc" | "ncat" | "telnet"
        | "invoke-webrequest" | "iwr" | "invoke-restmethod" | "irm" => {
            add(Risk::UsesNetwork);
            if any_arg(
                command,
                &["-o", "-O", "--output", "--remote-name", "-OutFile"],
            ) {
                add(Risk::ChangesFiles);
            }
        }
        "mv" | "cp" | "mkdir" | "touch" | "tee" | "ln" | "install" | "move-item" | "copy-item"
        | "new-item" | "set-content" | "add-content" | "out-file" | "move" | "copy" | "md"
        | "mkdirs" | "patch" | "unzip" | "zip" | "tar" | "7z" => add(Risk::ChangesFiles),
        "sed" | "gsed" if has_short_flag(command, 'i') || command.has_arg("--in-place") => {
            add(Risk::ChangesFiles);
        }
        "find" if command.has_arg("-delete") => add(Risk::DeletesFiles),
        "git" => git_risks(command, &mut add),
        "apt" | "apt-get" | "dnf" | "yum" | "pacman" | "zypper" | "snap" | "flatpak" | "brew"
        | "port" | "winget" | "choco" | "scoop" | "pip" | "pip3" | "pipx" | "gem" | "npm"
        | "pnpm" | "yarn" | "cargo" | "go" | "uv" | "poetry" | "conda" | "dotnet" => {
            package_risks(name, command, &mut add);
        }
        _ => {}
    }
    if LAUNCHERS.contains(&name)
        || (INTERPRETERS.contains(&name) && interpreter_rule(command).is_none())
    {
        add(Risk::RunsOtherCode);
    }
}

fn git_risks(command: &SimpleCommand, add: &mut impl FnMut(Risk)) {
    match subcommand(command) {
        Some("push" | "pull" | "fetch" | "clone" | "ls-remote") => add(Risk::UsesNetwork),
        Some("reset" | "clean" | "rm") if !command.has_arg("--soft") => add(Risk::DeletesFiles),
        Some(
            "commit" | "add" | "checkout" | "switch" | "merge" | "rebase" | "stash" | "restore"
            | "apply" | "cherry-pick" | "revert" | "tag" | "mv" | "init",
        ) => add(Risk::ChangesFiles),
        _ => {}
    }
    if git_is_destructive(command) {
        add(Risk::DeletesFiles);
    }
}

fn package_risks(name: &str, command: &SimpleCommand, add: &mut impl FnMut(Risk)) {
    let installs = matches!(
        subcommand(command),
        Some(
            "install"
                | "i"
                | "add"
                | "remove"
                | "rm"
                | "uninstall"
                | "upgrade"
                | "update"
                | "purge"
                | "autoremove"
                | "ci"
                | "sync"
                | "get"
                | "-S"
                | "-R"
                | "-Syu"
        )
    ) || (name == "pacman"
        && command
            .args()
            .first()
            .is_some_and(|w| w.text.starts_with("-S") || w.text.starts_with("-R")));
    if installs {
        add(Risk::InstallsSoftware);
        add(Risk::UsesNetwork);
    }
}

/// `curl … | sh`: something downloaded flows straight into a shell or interpreter.
pub(crate) fn pipes_download_into_code(commands: &[SimpleCommand]) -> bool {
    commands.windows(2).any(|pair| {
        let from = pair[0].program_name();
        let to = pair[1].program_name();
        pair[0].joiner == super::parse::Joiner::Pipe
            && matches!(
                from.as_str(),
                "curl" | "wget" | "iwr" | "invoke-webrequest" | "irm" | "invoke-restmethod"
            )
            && (LAUNCHERS.contains(&to.as_str()) || INTERPRETERS.contains(&to.as_str()))
    })
}
