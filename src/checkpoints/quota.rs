//! Logical archive usage and bounded admission for isolated test fixtures.
//! Production uses `u64::MAX` to leave retention to the GC policy.
use std::fs;
use std::io;
use std::path::{Component, Path};

#[cfg(test)]
pub const LIMIT: u64 = 1 << 30;
// Ordinary appends leave room for a pending operation's completion and the
// compaction move. A reserved append still has to fit within LIMIT.
pub const RESERVE: u64 = 64 << 10;
const FULL: &str = "Checkpoint archive capacity reached";

pub fn is_full(error: &str) -> bool {
    error.starts_with(FULL)
}

/// Only these files are created through `locks::open`, which never writes
/// file content. Other `.lock` files may contain data and use a fresh stat.
fn coordination_lock(archive: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(archive) else {
        return false;
    };
    let mut parts = relative.components();
    let Some(Component::Normal(first)) = parts.next() else {
        return false;
    };
    let Some(first) = first.to_str() else {
        return false;
    };
    let second = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let unique = |name: &str, prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|name| name.strip_suffix(".lock"))
            .is_some_and(|numbers| {
                let parts: Vec<_> = numbers.split('-').collect();
                parts.len() == 3
                    && parts.iter().all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
    };
    match second {
        None => {
            first == "store.lock"
                || first == "gc.lock"
                || first
                    .strip_prefix("gen-")
                    .and_then(|digits| digits.strip_suffix(".lock"))
                    .is_some_and(|digits| {
                        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
                    })
        }
        Some(Component::Normal(name)) => match (first, name.to_str()) {
            ("leases", Some(name)) => name.strip_suffix(".lock").is_some_and(|stripe| {
                stripe.len() == 2
                    && stripe
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }),
            ("owners", Some(name)) => unique(name, "owner-"),
            ("activity", Some(name)) => unique(name, "session-"),
            _ => false,
        },
        _ => false,
    }
}

enum MeasuredEntry {
    Directory,
    File(u64),
    Skip,
}

#[derive(Clone, Copy, Debug)]
pub struct Usage {
    pub total: u64,
    pub generation: u64,
}

fn present<T>(result: io::Result<T>, coordination: bool) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if coordination && error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not measure checkpoint archive: {error}")),
    }
}

fn measure_entry(
    archive: &Path,
    path: &Path,
    entry: &fs::DirEntry,
) -> Result<MeasuredEntry, String> {
    let coordination = coordination_lock(archive, path);
    let Some(kind) = present(entry.file_type(), coordination)? else {
        return Ok(MeasuredEntry::Skip);
    };
    if kind.is_dir() {
        return Ok(MeasuredEntry::Directory);
    }
    if !kind.is_file() {
        return Ok(MeasuredEntry::Skip);
    }
    // Windows may cache DirEntry metadata. Only Jecode's immutable
    // coordination locks can use it; even those get a fresh stat when
    // enumeration reports unexpected content. Unique owner locks may vanish
    // outside the store lock, so a missing canonical lock contributes zero.
    let cached = if coordination {
        let Some(metadata) = present(entry.metadata(), true)? else {
            return Ok(MeasuredEntry::Skip);
        };
        Some(metadata.len())
    } else {
        None
    };
    let length = if cached == Some(0) {
        0
    } else {
        let Some(metadata) = present(fs::metadata(path), coordination)? else {
            return Ok(MeasuredEntry::Skip);
        };
        metadata.len()
    };
    Ok(MeasuredEntry::File(length))
}

/// One walk counts both the whole archive and the active generation. Every
/// old generation, temporary file and reader-held generation counts in total.
pub fn measure(archive: &Path, generation: u64) -> Result<Usage, String> {
    fn visit(
        archive: &Path,
        directory: &Path,
        active: &Path,
        usage: &mut Usage,
    ) -> Result<(), String> {
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("Could not measure checkpoint archive: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("Could not measure checkpoint archive: {error}"))?;
            let path = entry.path();
            match measure_entry(archive, &path, &entry)? {
                MeasuredEntry::Directory => visit(archive, &path, active, usage)?,
                MeasuredEntry::File(length) => {
                    usage.total = usage
                        .total
                        .checked_add(length)
                        .ok_or("Checkpoint archive size overflow")?;
                    if path.starts_with(active) {
                        usage.generation = usage
                            .generation
                            .checked_add(length)
                            .ok_or("Checkpoint archive size overflow")?;
                    }
                }
                MeasuredEntry::Skip => {}
            }
        }
        Ok(())
    }
    let mut usage = Usage {
        total: 0,
        generation: 0,
    };
    let active = super::journal::generation_directory(archive, generation);
    visit(archive, archive, &active, &mut usage)?;
    Ok(usage)
}

/// Logical bytes owned by an archive, including all generations.
pub fn used(archive: &Path) -> Result<u64, String> {
    Ok(measure(archive, 0)?.total)
}

/// An active generation leaves half the archive for a compacted replacement.
/// Only terminal metadata can use the space held back for pending completions.
pub fn admit_generation(
    usage: Usage,
    added: u64,
    metadata_only: bool,
    terminal: bool,
    limit: u64,
) -> Result<(), String> {
    if limit == u64::MAX {
        return usage
            .total
            .checked_add(added)
            .zip(usage.generation.checked_add(added))
            .map(|_| ())
            .ok_or_else(|| "Checkpoint archive byte accounting overflow".into());
    }
    admit_measured(usage.total, added, terminal, limit)?;
    let working = limit / 2;
    let ceiling = if metadata_only && terminal {
        working
    } else {
        working.saturating_sub(RESERVE.min(working / 4))
    };
    if usage
        .generation
        .checked_add(added)
        .is_some_and(|total| total <= ceiling)
    {
        Ok(())
    } else {
        Err(format!(
            "{FULL}: active generation has {} used, {} requested, {} available",
            super::policy::size(usage.generation),
            super::policy::size(added),
            super::policy::size(ceiling.saturating_sub(usage.generation))
        ))
    }
}

pub fn admit(archive: &Path, added: u64, reserved: bool, limit: u64) -> Result<(), String> {
    if limit == u64::MAX {
        return Ok(());
    }
    let used = used(archive)?;
    admit_measured(used, added, reserved, limit)
}

/// Reuse a measurement only while the caller keeps the archive store lock.
pub fn admit_measured(used: u64, added: u64, reserved: bool, limit: u64) -> Result<(), String> {
    if limit == u64::MAX {
        return used
            .checked_add(added)
            .map(|_| ())
            .ok_or_else(|| "Checkpoint archive byte accounting overflow".into());
    }
    let ceiling = if reserved {
        limit
    } else {
        limit.saturating_sub(RESERVE.min(limit / 4))
    };
    if used
        .checked_add(added)
        .is_some_and(|total| total <= ceiling)
    {
        Ok(())
    } else {
        Err(format!(
            "{FULL}: {} used, {} requested, {} available",
            super::policy::size(used),
            super::policy::size(added),
            super::policy::size(ceiling.saturating_sub(used))
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;

    #[test]
    fn counts_all_owned_file_lengths_and_admits_before_the_limit() {
        let directory = crate::test_support::Directory::new();
        let archive = directory.path();
        fs::create_dir(archive.join("gen-000001")).unwrap();
        fs::create_dir(archive.join("gen-000002")).unwrap();
        fs::write(archive.join("gen-000001/000001.log"), vec![0; 300_000]).unwrap();
        fs::write(archive.join("gen-000002/000001.log"), vec![0; 1024]).unwrap();
        fs::write(archive.join("CURRENT.tmp"), vec![0; 32]).unwrap();
        assert_eq!(used(archive).unwrap(), 301_056);
        let measured = measure(archive, 2).unwrap();
        assert_eq!(measured.total, 301_056);
        assert_eq!(measured.generation, 1024);

        let limit = 301_056 + RESERVE + 100;
        assert!(admit(archive, 100, false, limit).is_ok());
        assert!(is_full(&admit(archive, 101, false, limit).unwrap_err()));
        assert!(admit(archive, RESERVE + 100, true, limit).is_ok());
        assert!(is_full(
            &admit(archive, RESERVE + 101, true, limit).unwrap_err()
        ));
    }

    #[test]
    fn nonterminal_frames_leave_headroom_for_completions() {
        let limit = 1 << 20;
        let usage = Usage {
            total: 700 << 10,
            generation: limit / 2 - RESERVE,
        };
        assert!(is_full(
            &admit_generation(usage, 1, false, false, limit).unwrap_err()
        ));
        assert!(is_full(
            &admit_generation(usage, 1, true, false, limit).unwrap_err()
        ));
        assert!(admit_generation(usage, RESERVE, true, true, limit).is_ok());
        assert!(is_full(
            &admit_generation(usage, RESERVE + 1, true, true, limit).unwrap_err()
        ));
        let held_old_generation = Usage {
            total: limit - RESERVE,
            generation: 0,
        };
        assert!(is_full(
            &admit_generation(held_old_generation, 1, false, false, limit).unwrap_err()
        ));
        assert!(admit_generation(held_old_generation, 1, true, true, limit).is_ok());
    }

    #[test]
    fn immutable_lock_shortcut_keeps_mutable_archive_lengths_fresh() {
        let directory = crate::test_support::Directory::new();
        let archive = directory.path();
        for name in ["leases", "owners", "activity", "gen-000001"] {
            fs::create_dir(archive.join(name)).unwrap();
        }
        for stripe in 0..256 {
            fs::File::create(archive.join(format!("leases/{stripe:02x}.lock"))).unwrap();
        }
        for name in [
            "store.lock",
            "gc.lock",
            "gen-000001.lock",
            "activity/session-1-2-3.lock",
        ] {
            fs::File::create(archive.join(name)).unwrap();
            assert!(coordination_lock(archive, &archive.join(name)), "{name}");
        }
        let owner = archive.join("owners/owner-1-2-3.lock");
        fs::write(&owner, b"old!").unwrap();
        assert!(coordination_lock(archive, &owner));
        assert!(!coordination_lock(archive, &archive.join("other.lock")));
        assert!(!coordination_lock(
            archive,
            &archive.join("leases/other.lock")
        ));

        let mut log = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(archive.join("gen-000001/000001.log"))
            .unwrap();
        let mut other = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(archive.join("other.lock"))
            .unwrap();
        log.write_all(&[7; 128]).unwrap();
        other.write_all(b"other").unwrap();
        assert_eq!(used(archive).unwrap(), 137);
        log.write_all(&[8; 64]).unwrap();
        other.write_all(b" changed!").unwrap();
        assert_eq!(used(archive).unwrap(), 210);
    }

    #[test]
    fn vanished_coordination_lock_counts_zero_but_other_missing_files_fail() {
        let directory = crate::test_support::Directory::new();
        let archive = directory.path();
        let owners = archive.join("owners");
        fs::create_dir(&owners).unwrap();
        let owner = owners.join("owner-1-2-3.lock");
        fs::File::create(&owner).unwrap();
        let entry = fs::read_dir(&owners).unwrap().next().unwrap().unwrap();
        fs::remove_file(&owner).unwrap();
        assert!(matches!(
            measure_entry(archive, &entry.path(), &entry).unwrap(),
            MeasuredEntry::Skip | MeasuredEntry::File(0)
        ));

        let other = owners.join("other.lock");
        fs::File::create(&other).unwrap();
        let entry = fs::read_dir(&owners).unwrap().next().unwrap().unwrap();
        fs::remove_file(&other).unwrap();
        assert!(measure_entry(archive, &entry.path(), &entry).is_err());
    }
}
