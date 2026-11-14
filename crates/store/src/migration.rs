//! Automatic migration from the former GTK client (not upstream Throne).
//!
//! Existing Nolurix directories always win: there is no merge or overwrite.
//! Only the main SQLite database and SQLite-format caches are allowlisted;
//! settings, groups, profiles, subscriptions and routing live in the main database.
//! sing-box's experimental cache normally uses BoltDB/bbolt, not SQLite, despite
//! its `.db` suffix. That regenerable cache (and any other non-SQLite cache) is
//! deliberately omitted: copying a live BoltDB file is not a safe snapshot.
//! Runtime state, helper
//! credentials, logs and arbitrary files are deliberately not imported.
//! Originals are never deleted. An interrupted private stage is ignored on retry.

use anyhow::{bail, Context, Result};
use rusqlite::{backup::Backup, Connection, OpenFlags};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

/// Resolve startup storage without importing default legacy data into an
/// explicit override (including an override equal to the default directory).
pub fn prepare_config_dir(
    old: &Path,
    new: &Path,
    explicit: Option<&Path>,
) -> Result<std::path::PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_owned());
    }
    migrate_legacy(old, new)?;
    Ok(new.to_owned())
}

/// Migrate only default storage. Callers with explicit directory overrides must
/// not call this function. Any failure must abort startup before opening a store.
pub fn migrate_legacy(old: &Path, new: &Path) -> Result<()> {
    checked_ancestors(new)?;
    if exists(new)? {
        if !fs::symlink_metadata(new)?.is_dir() {
            bail!("Nolurix storage is not a directory: {}", new.display());
        }
        check_destination(new)?;
        return Ok(());
    }
    checked_ancestors(old)?;
    if !exists(old)? {
        return Ok(());
    }
    if !fs::symlink_metadata(old)?.is_dir() {
        bail!("legacy GTK storage is not a directory: {}", old.display());
    }
    let main = old.join("throne-gtk.db");
    if !exists(&main)? {
        // An old directory with no database is not user data to migrate.
        return Ok(());
    }
    let parent = new.parent().context("storage directory has no parent")?;
    fs::create_dir_all(parent)?;
    checked_ancestors(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".nolurix-migration-")
        .tempdir_in(parent)?;
    snapshot(&main, &stage.path().join("nolurix.db"))?;
    let cache = old.join("cache.db");
    if exists(&cache)? && sqlite_cache(&cache)? {
        snapshot(&cache, &stage.path().join("cache.db"))?;
    }
    sync_dir(stage.path())?;
    // Recheck before publication, including destination symlinks. rename alone
    // would overwrite an empty destination directory on Unix.
    checked_ancestors(new)?;
    match publish(stage.path(), new) {
        Ok(()) => sync_dir(parent)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            checked_ancestors(new)?;
            if !fs::symlink_metadata(new)?.is_dir() {
                bail!("concurrent Nolurix destination is not a directory");
            }
            check_destination(new)?;
        }
        Err(e) => return Err(e).context("publishing migrated GTK storage"),
    }
    Ok(())
}

fn sqlite_cache(path: &Path) -> Result<bool> {
    checked_ancestors(path)?;
    if !fs::symlink_metadata(path)?.is_file() {
        bail!("legacy cache is not a regular file: {}", path.display());
    }
    let mut header = [0_u8; 16];
    match fs::File::open(path)?.read_exact(&mut header) {
        Ok(()) => Ok(&header == b"SQLite format 3\0"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e).context("identifying legacy cache format"),
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn check_destination(dir: &Path) -> Result<()> {
    for name in ["nolurix.db", "cache.db"] {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let path = dir.join(format!("{name}{suffix}"));
            checked_ancestors(&path)?;
            if exists(&path)? && !fs::symlink_metadata(&path)?.is_file() {
                bail!(
                    "Nolurix SQLite file is not a regular file: {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn checked_ancestors(path: &Path) -> Result<()> {
    for part in path.ancestors() {
        if part.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(part) {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("refusing symlink in migration path: {}", part.display())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", part.display())),
        }
    }
    Ok(())
}

fn snapshot(source: &Path, target: &Path) -> Result<()> {
    checked_ancestors(source)?;
    if !fs::symlink_metadata(source)?.is_file() {
        bail!(
            "legacy database is not a regular file: {}",
            source.display()
        );
    }
    // SQLite may open these itself; never follow an attacker-supplied sidecar.
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = source.as_os_str().to_owned();
        sidecar.push(suffix);
        let sidecar = Path::new(&sidecar);
        checked_ancestors(sidecar)?;
        if exists(sidecar)? && !fs::symlink_metadata(sidecar)?.is_file() {
            bail!("legacy SQLite sidecar is not a regular file");
        }
    }
    let src = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut dst = Connection::open(target)?;
    {
        let backup = Backup::new(&src, &mut dst)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match backup.step(128)? {
                rusqlite::backup::StepResult::Done => break,
                _ if Instant::now() >= deadline => bail!("legacy database backup timed out"),
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    let integrity: String = dst.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        bail!("legacy database integrity check failed: {integrity}");
    }
    dst.close().map_err(|(_, e)| e)?;
    fs::File::open(target)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_dir(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// Windows directory rename fails if the destination already exists.
#[cfg(windows)]
fn publish(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn publish(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-clobber migration is unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        // macOS commonly exposes TMPDIR through /var -> /private/var. Tests
        // use the physical parent; migration itself never follows symlinks.
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }

    fn fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, Connection) {
        let old = root.join("throne-gtk");
        fs::create_dir(&old).unwrap();
        let conn = Connection::open(old.join("throne-gtk.db")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE state(value TEXT); INSERT INTO state VALUES ('preserved');",
        )
        .unwrap();
        (old, root.join("nolurix"), conn)
    }

    fn value(path: &Path) -> String {
        Connection::open(path.join("nolurix.db"))
            .unwrap()
            .query_row("SELECT value FROM state", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn fresh_no_legacy_does_not_create_storage() {
        let root = temp_root();
        let new = root.path().join("nolurix");
        migrate_legacy(&root.path().join("throne-gtk"), &new).unwrap();
        assert!(!new.exists());
    }

    #[test]
    fn live_wal_main_cache_repeat_and_allowlist() {
        let root = temp_root();
        let (old, new, live) = fixture(root.path());
        let cache = Connection::open(old.join("cache.db")).unwrap();
        cache
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE cache(value); INSERT INTO cache VALUES (42);",
            )
            .unwrap();
        fs::write(old.join("helper-token"), "secret").unwrap();
        fs::write(old.join("core.log"), "log").unwrap();
        migrate_legacy(&old, &new).unwrap();
        assert_eq!(value(&new), "preserved");
        let n: i64 = Connection::open(new.join("cache.db"))
            .unwrap()
            .query_row("SELECT value FROM cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 42);
        assert!(!new.join("helper-token").exists());
        assert!(!new.join("core.log").exists());
        live.execute("UPDATE state SET value='later'", []).unwrap();
        migrate_legacy(&old, &new).unwrap();
        assert_eq!(value(&new), "preserved");
        assert!(old.join("throne-gtk.db").exists());
    }

    #[test]
    fn existing_empty_directory_wins_without_merge() {
        let root = temp_root();
        let (old, new, _live) = fixture(root.path());
        fs::create_dir(&new).unwrap();
        migrate_legacy(&old, &new).unwrap();
        assert_eq!(fs::read_dir(new).unwrap().count(), 0);
    }

    #[test]
    fn explicit_overrides_never_import_default_legacy_data() {
        let root = temp_root();
        let (old, new, _live) = fixture(root.path());
        let custom = root.path().join("custom");
        assert_eq!(
            prepare_config_dir(&old, &new, Some(&custom)).unwrap(),
            custom
        );
        assert!(!custom.exists());
        assert!(!new.exists());
        assert_eq!(prepare_config_dir(&old, &new, Some(&new)).unwrap(), new);
        assert!(!new.exists());
        prepare_config_dir(&old, &new, None).unwrap();
        assert_eq!(value(&new), "preserved");
    }

    #[test]
    fn actual_store_schema_settings_subscriptions_profiles_and_routes_survive() {
        use crate::Store;
        use nolurix_config::{profile::Group, route::RouteRule};
        let root = temp_root();
        let old = root.path().join("throne-gtk");
        let new = root.path().join("nolurix");
        let mut source = Store::open(&old.join("throne-gtk.db")).unwrap();
        let gid = source
            .insert_group(&Group {
                name: "subscription".into(),
                url: "https://example.test/sub".into(),
                ..Default::default()
            })
            .unwrap();
        let mut profile = nolurix_config::link::parse("trojan://pw@a.example:443#saved").unwrap();
        profile.gid = gid;
        let pid = source.insert_profile(&profile).unwrap();
        let mut settings = source.settings().unwrap();
        settings.mixed_port = 12345;
        settings.last_profile_id = pid;
        settings.route_rules.push(RouteRule::default());
        source.save_settings(&settings).unwrap();
        migrate_legacy(&old, &new).unwrap();
        let migrated = Store::open(&new.join("nolurix.db")).unwrap();
        assert_eq!(
            serde_json::to_value(migrated.settings().unwrap()).unwrap(),
            serde_json::to_value(&settings).unwrap()
        );
        assert_eq!(
            migrated.group(gid).unwrap().unwrap().url,
            "https://example.test/sub"
        );
        let saved = migrated.profile(pid).unwrap().unwrap();
        assert_eq!(saved.gid, gid);
        assert_eq!(saved.outbound, profile.outbound);
        assert_eq!(source.profile(pid).unwrap().unwrap().name, "saved");
    }

    #[test]
    fn failure_and_abandoned_stage_retry() {
        let root = temp_root();
        let (old, new, _live) = fixture(root.path());
        fs::write(old.join("cache.db"), b"SQLite format 3\0invalid").unwrap();
        assert!(migrate_legacy(&old, &new).is_err());
        assert!(!new.exists());
        fs::remove_file(old.join("cache.db")).unwrap();
        let abandoned = root.path().join(".nolurix-migration-crashed");
        fs::create_dir(&abandoned).unwrap();
        fs::write(abandoned.join("nolurix.db"), "partial").unwrap();
        migrate_legacy(&old, &new).unwrap();
        assert_eq!(value(&new), "preserved");
    }

    #[test]
    fn disposable_bbolt_and_unknown_caches_are_omitted_without_blocking_state() {
        for contents in [
            // BoltDB meta page begins with page ID/flags; magic follows at
            // offset 16 rather than a SQLite header.
            [vec![0_u8; 16], vec![0xed, 0xda, 0x0c, 0xed], vec![0_u8; 64]].concat(),
            b"unknown regenerable cache".to_vec(),
            Vec::new(),
        ] {
            let root = temp_root();
            let (old, new, _live) = fixture(root.path());
            fs::write(old.join("cache.db"), &contents).unwrap();
            migrate_legacy(&old, &new).unwrap();
            assert_eq!(value(&new), "preserved");
            assert!(!new.join("cache.db").exists());
            assert_eq!(fs::read(old.join("cache.db")).unwrap(), contents);
        }
    }

    #[test]
    fn concurrent_starts_and_atomic_no_clobber() {
        let root = temp_root();
        let (old, new, _live) = fixture(root.path());
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| migrate_legacy(&old, &new)))
                .collect();
            for job in jobs {
                job.join().unwrap().unwrap();
            }
        });
        assert_eq!(value(&new), "preserved");
        let stage = root.path().join("stage");
        let occupied = root.path().join("occupied");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("marker"), "not published").unwrap();
        fs::create_dir(&occupied).unwrap();
        assert_eq!(
            publish(&stage, &occupied).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert!(!occupied.join("marker").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_errors_not_fresh_starts() {
        use std::os::unix::fs::symlink;
        let root = temp_root();
        let (old, new, _live) = fixture(root.path());
        symlink(&old, &new).unwrap();
        assert!(migrate_legacy(&old, &new).is_err());
        fs::remove_file(&new).unwrap();
        symlink(old.join("throne-gtk.db"), old.join("cache.db")).unwrap();
        assert!(migrate_legacy(&old, &new).is_err());
        assert!(!new.exists());
        fs::remove_file(old.join("cache.db")).unwrap();
        fs::create_dir(&new).unwrap();
        symlink(old.join("throne-gtk.db"), new.join("nolurix.db")).unwrap();
        assert!(migrate_legacy(&old, &new).is_err());
    }
}
