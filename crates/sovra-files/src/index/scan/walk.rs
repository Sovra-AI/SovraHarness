//! Stage 1: a single capability-safe walk that only *finds* work. It opens each regular
//! file (no symlink following) and hands the handle to the readers, so the expensive part
//! — reading, hashing, extracting — runs in parallel while the walk keeps counting.
//!
//! One walker is deliberate: directory iteration is cheap next to extraction, an explicit
//! stack keeps only the current branch's handles open, and every path stays relative to a
//! held capability. The bounded job channel caps open file handles at `2 × readers`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::SyncSender;

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsSyncExt as _};
use cap_std::fs::{Dir, OpenOptions};
use sovra_core::file_index::{ExtractionState, NoticeKind, ScanOptions};

use super::{Job, Outcome, Shared, Tally};

/// Walk `dir`, sending files to `jobs` and walk-level notices to `out`. Returns how many
/// folders or entries could not be read; a non-zero count makes the scan incomplete.
pub(super) fn walk(
    dir: Dir,
    options: ScanOptions,
    excluded: Option<&Path>,
    shared: &Shared<'_>,
    jobs: &SyncSender<Job>,
    out: &SyncSender<Outcome>,
) -> u64 {
    let notice = |kind, path: &str, message: &str, tally| {
        out.send(Outcome::Notice {
            kind,
            path: path.into(),
            message: message.into(),
            tally,
        })
        .is_ok()
    };
    let traversal = |path: &Path, error: &std::io::Error| {
        let reason = match error.kind() {
            std::io::ErrorKind::PermissionDenied => "Access denied. Check folder permissions and allow Sovra to read it, then refresh to retry.".into(),
            std::io::ErrorKind::NotFound => "This entry disappeared while scanning. Refresh to check the folder again.".into(),
            kind => format!("This folder or entry could not be read ({kind}). Check access and availability, then refresh to retry."),
        };
        let label = if path.as_os_str().is_empty() {
            ".".into()
        } else {
            path.to_string_lossy()
        };
        notice(NoticeKind::Failed, &label, &reason, Tally::Failed)
    };
    let Ok(entries) = dir.entries().inspect_err(|e| {
        traversal(Path::new(""), e);
    }) else {
        return 1;
    };
    // An explicit stack avoids a call-stack depth cutoff. Each directory is held as a
    // capability; only the current branch has open handles.
    let mut stack = vec![(entries, dir, PathBuf::new())];
    let mut unread_entries = 0_u64;
    while let Some((entries, dir, relative)) = stack.last_mut() {
        if shared.stopped() {
            return unread_entries;
        }
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        shared.visited.fetch_add(1, Relaxed);
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                unread_entries += 1;
                if !traversal(relative, &error) {
                    return unread_entries;
                }
                // An iterator error leaves the rest of this directory uncertain. Continue
                // from its parent rather than polling a broken iterator.
                stack.pop();
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            let escaped = super::escaped_path(&relative.join(&name));
            if !notice(
                NoticeKind::Skipped,
                &escaped,
                "Filename is not valid Unicode; its path is shown with escapes. Rename it to include it in the index.",
                Tally::Skipped,
            ) {
                return unread_entries;
            }
            continue;
        };
        let path = relative.join(name);
        if let Some(reason) = exclusion(&path, name, options, excluded) {
            if !notice(
                NoticeKind::Skipped,
                &path.to_string_lossy(),
                reason,
                Tally::Skipped,
            ) {
                return unread_entries;
            }
            continue;
        }
        let metadata = match dir.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) => {
                unread_entries += 1;
                if !traversal(&path, &error) {
                    return unread_entries;
                }
                continue;
            }
        };
        let sent = if metadata.is_symlink() {
            notice(
                NoticeKind::Skipped,
                &path.to_string_lossy(),
                "Symbolic link; add its destination folder directly to include it.",
                Tally::Skipped,
            )
        } else if metadata.is_dir() {
            // Every component is opened relative to a held capability and rejects
            // symlinks, including a link swapped in during traversal.
            match dir
                .open_dir_nofollow(name)
                .and_then(|child| child.entries().map(|entries| (entries, child)))
            {
                Ok((entries, child)) => {
                    stack.push((entries, child, path));
                    true
                }
                Err(error) => {
                    unread_entries += 1;
                    traversal(&path, &error)
                }
            }
        } else if metadata.is_file() {
            let Some(relative) = path.to_str() else {
                continue;
            };
            let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
            shared.discovered.fetch_add(1, Relaxed);
            let mut open = OpenOptions::new();
            open.read(true).follow(FollowSymlinks::No).nonblock(true);
            let job = dir
                .open_with(name, &open)
                .and_then(|file| file.metadata().map(|before| (file, before)));
            match job {
                Ok((file, before)) if before.is_file() => jobs
                    .send(Job {
                        file,
                        name: name.to_owned(),
                        path: relative,
                        metadata: before,
                    })
                    .is_ok(),
                Ok(_) => out
                    .send(Outcome::Metadata {
                        path: relative,
                        metadata,
                        state: ExtractionState::Failed,
                        note: "Only regular files can be indexed.".into(),
                        notice: NoticeKind::Failed,
                        tally: Tally::Failed,
                    })
                    .is_ok(),
                Err(error) => out
                    .send(Outcome::Metadata {
                        path: relative,
                        metadata,
                        state: ExtractionState::Failed,
                        note: error.to_string(),
                        notice: NoticeKind::Failed,
                        tally: Tally::Failed,
                    })
                    .is_ok(),
            }
        } else {
            notice(
                NoticeKind::Skipped,
                &path.to_string_lossy(),
                "Not a regular file or folder.",
                Tally::Skipped,
            )
        };
        if !sent {
            return unread_entries;
        }
    }
    shared.walk_done.store(true, Relaxed);
    unread_entries
}

/// Why an entry is left out, shared by the walk and the count so both agree on the total.
fn exclusion(
    path: &Path,
    name: &str,
    options: ScanOptions,
    excluded: Option<&Path>,
) -> Option<&'static str> {
    if excluded.is_some_and(|p| path == p) {
        Some("Sovra's own index data")
    } else {
        options.exclusion_reason(name)
    }
}

/// A listing-only pass that finds the total number of files early, so the person sees a
/// real "1,204 of 5,000" bar long before the reading walk (which waits on the readers)
/// has reached the end. It opens no files and reports nothing; the reading walk remains
/// the authority for notices and for what counts as complete.
pub(super) fn count(dir: Dir, options: ScanOptions, excluded: Option<&Path>, shared: &Shared<'_>) {
    let Ok(entries) = dir.entries() else { return };
    let mut stack = vec![(entries, dir, PathBuf::new())];
    while let Some((entries, dir, relative)) = stack.last_mut() {
        if shared.stopped() || shared.walk_done.load(Relaxed) {
            return;
        }
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        let Ok(entry) = entry else {
            stack.pop();
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = relative.join(name);
        if exclusion(&path, name, options, excluded).is_some() {
            continue;
        }
        let Ok(metadata) = dir.symlink_metadata(name) else {
            continue;
        };
        if metadata.is_dir() && !metadata.is_symlink() {
            if let Ok((entries, child)) = dir
                .open_dir_nofollow(name)
                .and_then(|child| child.entries().map(|entries| (entries, child)))
            {
                stack.push((entries, child, path));
            }
        } else if metadata.is_file() {
            shared.counted.fetch_add(1, Relaxed);
        }
    }
    shared.count_done.store(true, Relaxed);
}
