//! What the marketplace keeps between runs, in its own folder inside the plugin's state folder: when
//! updates were last checked, the lock that keeps two checks from running at once, the updates the
//! user said no to, and when the explorer was last opened. The time comes in as a parameter so the
//! rules can be tested.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use common::fs_util::write_atomic;

const LAST_CHECK: &str = "last-check";
const LOCK: &str = "check.lock";
const DISMISSED: &str = "dismissed.json";
const LAST_OPEN: &str = "last-open";
/// Updates found by the check, read by the updates window.
pub const PENDING: &str = "pending.json";

/// A check that has held the lock this long died without releasing it.
const LOCK_EXPIRY_SECS: u64 = 10 * 60;

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether the periodic check is due. `every_hours == 0` turns it off.
pub fn due(dir: &Path, every_hours: u8, now: SystemTime) -> bool {
    if every_hours == 0 {
        return false;
    }
    match fs::read_to_string(dir.join(LAST_CHECK)).ok().and_then(|t| t.trim().parse::<u64>().ok()) {
        Some(last) => secs(now).saturating_sub(last) >= u64::from(every_hours) * 3600,
        None => true,
    }
}

pub fn mark_checked(dir: &Path, now: SystemTime) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(LAST_CHECK), secs(now).to_string().as_bytes())
}

/// Held while a check runs; dropping it frees the lock, but only if it is still ours.
pub struct Lock {
    path: PathBuf,
    token: String,
}

impl Drop for Lock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|t| t == self.token) {
            // If it cannot be removed it expires on its own after LOCK_EXPIRY_SECS.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// `None` if another check holds the lock. Several focus events arrive almost together when switching
/// workspace, and each one launches a check: only the first one gets through.
pub fn try_lock(dir: &Path, now: SystemTime) -> io::Result<Option<Lock>> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(dir)?;
    let path = dir.join(LOCK);
    let token = format!("{} {} {}", secs(now), std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        // Written through the same handle: the file exists before its token does, and a reader in
        // between sees it empty, which counts as held (below).
        Ok(mut file) => io::Write::write_all(&mut file, token.as_bytes())?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let since = fs::read_to_string(&path)
                .ok()
                .and_then(|t| t.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()));
            let expired = match since {
                Some(since) => secs(now).saturating_sub(since) >= LOCK_EXPIRY_SECS,
                // Empty or unreadable: being written right now, unless it has been like that too long.
                None => fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|age| age.as_secs() >= LOCK_EXPIRY_SECS)),
            };
            if !expired {
                return Ok(None);
            }
            write_atomic(&path, token.as_bytes())?;
        }
        Err(e) => return Err(e),
    }
    Ok(Some(Lock { path, token }))
}

/// The check found updates the window has not shown yet.
pub fn has_pending(dir: &Path) -> bool {
    dir.join(PENDING).is_file()
}

/// At herdr startup: what a previous server found and could not show is looked for again.
pub fn forget_pending(dir: &Path) -> io::Result<()> {
    match fs::remove_file(dir.join(PENDING)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Removes the list of pending updates if it still is `original`: a newer check may have replaced it
/// while the window showed the old one.
pub fn remove_pending_if(dir: &Path, original: &str) -> io::Result<()> {
    match fs::read_to_string(dir.join(PENDING)) {
        Ok(now) if now == original => fs::remove_file(dir.join(PENDING)),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Updates the user said no to, as (plugin id, commit offered). They are not offered again until the
/// plugin publishes a newer commit or herdr restarts.
pub fn dismissed(dir: &Path) -> Vec<(String, String)> {
    fs::read_to_string(dir.join(DISMISSED)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn dismiss(dir: &Path, items: &[(String, String)]) -> io::Result<()> {
    let mut all = dismissed(dir);
    for item in items {
        if !all.contains(item) {
            all.push(item.clone());
        }
    }
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(DISMISSED), serde_json::to_string(&all).map_err(io::Error::other)?.as_bytes())
}

/// At herdr startup everything is offered again.
pub fn forget_dismissed(dir: &Path) -> io::Result<()> {
    match fs::remove_file(dir.join(DISMISSED)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// When the explorer was last opened (RFC 3339), to mark what is new since then.
pub fn last_open(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(LAST_OPEN)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

pub fn set_last_open(dir: &Path, now: SystemTime) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(LAST_OPEN), rfc3339(now).as_bytes())
}

/// UTC, to the second: the same format as the index's dates, so the two compare as text.
pub fn rfc3339(t: SystemTime) -> String {
    let total = secs(t);
    let (days, rest) = ((total / 86_400) as i64, total % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    pub fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-market-tests-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    const HOUR: u64 = 3600;

    #[test]
    fn first_time_is_due() {
        assert!(due(&folder("first"), 6, at(1_000_000)));
    }

    #[test]
    fn due_after_interval() {
        let dir = folder("interval");
        mark_checked(&dir, at(1_000_000)).unwrap();
        assert!(!due(&dir, 6, at(1_000_000 + 6 * HOUR - 60)));
        assert!(due(&dir, 6, at(1_000_000 + 6 * HOUR)));
    }

    #[test]
    fn zero_never_due() {
        assert!(!due(&folder("zero"), 0, at(1_000_000)));
    }

    #[test]
    fn lock_is_exclusive_and_expires() {
        let dir = folder("lock");
        let first = try_lock(&dir, at(1_000_000)).unwrap();
        assert!(first.is_some());
        assert!(try_lock(&dir, at(1_000_000 + 60)).unwrap().is_none());
        // A lock older than 10 minutes belongs to a check that died: it is taken over.
        let taken_over = try_lock(&dir, at(1_000_000 + 11 * 60)).unwrap();
        assert!(taken_over.is_some());
        // The dead check finishing late must not free the lock it no longer owns.
        drop(first);
        assert!(try_lock(&dir, at(1_000_000 + 12 * 60)).unwrap().is_none());
        drop(taken_over);
        assert!(try_lock(&dir, at(1_000_000 + 12 * 60)).unwrap().is_some());
    }

    #[test]
    fn empty_lock_counts_as_held() {
        // Another check has just created the file and not written its token yet.
        let dir = folder("lock-empty");
        fs::write(dir.join("check.lock"), "").unwrap();
        assert!(try_lock(&dir, SystemTime::now()).unwrap().is_none());
    }

    #[test]
    fn pending_file() {
        let dir = folder("pending");
        assert!(!has_pending(&dir));
        fs::write(dir.join(PENDING), "[]").unwrap();
        assert!(has_pending(&dir));
    }

    #[test]
    fn pending_is_removed_only_if_unchanged() {
        let dir = folder("pending-unchanged");
        fs::write(dir.join(PENDING), "[1]").unwrap();
        remove_pending_if(&dir, "[1]").unwrap();
        assert!(!has_pending(&dir));
        // A newer check wrote another list while the window was open: it stays.
        fs::write(dir.join(PENDING), "[2]").unwrap();
        remove_pending_if(&dir, "[1]").unwrap();
        assert!(has_pending(&dir));
    }

    #[test]
    fn dropping_the_lock_frees_it() {
        let dir = folder("lock-drop");
        drop(try_lock(&dir, at(1_000_000)).unwrap());
        assert!(try_lock(&dir, at(1_000_001)).unwrap().is_some());
    }

    #[test]
    fn dismissed_round_trip() {
        let dir = folder("dismissed");
        assert!(dismissed(&dir).is_empty());
        let items = vec![("a".to_string(), "1111".to_string()), ("b".to_string(), "2222".to_string())];
        dismiss(&dir, &items).unwrap();
        dismiss(&dir, &[("a".to_string(), "1111".to_string())]).unwrap(); // no duplicates
        assert_eq!(dismissed(&dir), items);
        forget_dismissed(&dir).unwrap();
        assert!(dismissed(&dir).is_empty());
    }

    #[test]
    fn last_open_round_trip() {
        let dir = folder("last-open");
        assert_eq!(last_open(&dir), None);
        set_last_open(&dir, at(1_792_771_821)).unwrap();
        assert_eq!(last_open(&dir).as_deref(), Some("2026-10-23T16:10:21Z"));
    }

    #[test]
    fn rfc3339_dates() {
        assert_eq!(rfc3339(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(at(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(at(1_792_771_821)), "2026-10-23T16:10:21Z");
    }
}
