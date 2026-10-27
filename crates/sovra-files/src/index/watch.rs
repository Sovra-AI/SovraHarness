//! Native notifications are hints. Only the capability-based scanner reads
//! source contents, and runtime periodically reconciles even a healthy watch.

use std::path::{Component, Path};
use std::sync::Arc;

use cap_fs_ext::DirExt as _;
use cap_std::fs::Dir;
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use sovra_core::file_index::{
    IndexChanges, IndexError, IndexWatch, IndexedRoot, MAX_INDEX_ENTRIES, excluded_name,
};

use super::io_error;

const MAX_WATCHED_DIRS: usize = 2_048;

struct NativeWatch(RecommendedWatcher);
impl IndexWatch for NativeWatch {}

pub(super) fn start(
    root: &IndexedRoot,
    storage: &Path,
    changes: Arc<dyn IndexChanges>,
) -> Result<Box<dyn IndexWatch>, IndexError> {
    let scope = root.path.clone();
    let excluded = std::fs::canonicalize(storage).ok();
    let event_scope = scope.clone();
    let event_excluded = excluded.clone();
    let mut guard = NativeWatch(
        RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| match event {
                Ok(event) if relevant(&event, &event_scope, event_excluded.as_deref()) => {
                    changes.changed();
                }
                Ok(_) => {}
                Err(_) => changes.failed(
                    "Native folder notifications stopped. Periodic checks remain active.".into(),
                ),
            },
            notify::Config::default().with_follow_symlinks(false),
        )
        .map_err(|_| {
            IndexError("Native folder notifications are unavailable. Using periodic checks.".into())
        })?,
    );
    let dir =
        Dir::open_ambient_dir(&scope, cap_std::ambient_authority()).map_err(|e| io_error(&e))?;
    let mut budget = Budget::default();
    let mut paths = guard.0.paths_mut();
    register(
        paths.as_mut(),
        &dir,
        &scope,
        excluded.as_deref(),
        0,
        &mut budget,
    )?;
    paths.commit().map_err(|_| {
        IndexError(
            "Native folder notifications could not start. Periodic checks remain active.".into(),
        )
    })?;
    Ok(Box::new(guard))
}

#[derive(Default)]
struct Budget {
    directories: usize,
    entries: u64,
}

fn register(
    watcher: &mut dyn notify::PathsMut,
    dir: &Dir,
    path: &Path,
    excluded: Option<&Path>,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), IndexError> {
    if budget.directories >= MAX_WATCHED_DIRS || depth > 64 {
        return Err(IndexError("This scope exceeds the native watch budget. Periodic checks remain active; choose smaller folders for faster updates.".into()));
    }
    watcher
        .add(path, RecursiveMode::NonRecursive)
        .map_err(|_| {
            IndexError(
                "The system could not watch every folder. Periodic checks remain active.".into(),
            )
        })?;
    budget.directories += 1;
    for entry in dir.entries().map_err(|e| io_error(&e))? {
        budget.entries += 1;
        if budget.entries > MAX_INDEX_ENTRIES {
            return Err(IndexError("This scope exceeds the notification discovery budget. Periodic checks remain active.".into()));
        }
        let entry = entry.map_err(|e| io_error(&e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let child_path = path.join(name);
        if excluded_name(name) || excluded.is_some_and(|excluded| child_path.starts_with(excluded))
        {
            continue;
        }
        let metadata = dir.symlink_metadata(name).map_err(|e| io_error(&e))?;
        if metadata.is_dir() && !metadata.is_symlink() {
            let child = dir.open_dir_nofollow(name).map_err(|e| io_error(&e))?;
            register(watcher, &child, &child_path, excluded, depth + 1, budget)?;
        }
    }
    Ok(())
}

fn relevant(event: &notify::Event, scope: &Path, excluded: Option<&Path>) -> bool {
    if event.need_rescan() {
        return true;
    }
    if event.kind.is_access() {
        return false;
    }
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            if excluded.is_some_and(|excluded| path.starts_with(excluded)) {
                return false;
            }
            let Ok(relative) = path.strip_prefix(scope) else {
                return false;
            };
            relative.components().all(|component| match component {
                Component::Normal(name) => name.to_str().is_some_and(|name| !excluded_name(name)),
                Component::CurDir
                | Component::ParentDir
                | Component::RootDir
                | Component::Prefix(_) => false,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_skip_noise_and_reads_but_keep_renames_and_rescan_flags() {
        use notify::event::{AccessKind, Flag, ModifyKind};
        let root = Path::new("/chosen");
        let excluded = Some(Path::new("/chosen/derived"));
        for path in [
            "/outside/a",
            "/chosen/.env",
            "/chosen/target/a",
            "/chosen/derived/a",
            "/chosen/../secret",
        ] {
            let event = notify::Event::new(notify::EventKind::Any).add_path(path.into());
            assert!(!relevant(&event, root, excluded), "{path}");
        }
        let rename = notify::Event::new(notify::EventKind::Modify(ModifyKind::Any))
            .add_path("/chosen/target/file".into())
            .add_path("/chosen/notes/file".into());
        assert!(relevant(&rename, root, excluded));
        let read = notify::Event::new(notify::EventKind::Access(AccessKind::Any))
            .add_path("/chosen/a".into());
        assert!(!relevant(&read, root, excluded));
        assert!(relevant(&read.set_flag(Flag::Rescan), root, excluded));
        assert!(relevant(
            &notify::Event::new(notify::EventKind::Any).add_path(root.into()),
            root,
            excluded
        ));
    }
}
