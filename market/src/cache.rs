//! herdr's plugin index kept on disk for 15 minutes, so opening and closing the explorer is instant
//! and it still has something to show without network.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::fs_util::write_atomic;

use crate::index::{self, Listing};

const FILE: &str = "index.json";
const TIME: &str = "index.time";
const TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, PartialEq)]
pub enum Freshness {
    Fresh,
    /// The download failed; this is the list saved at that time.
    Stale(SystemTime),
}

/// The cached index if it is younger than 15 minutes; otherwise `fetch`. If `fetch` fails or brings
/// something that does not parse, the old copy is used and said so; without one, the error.
pub fn load_index(
    dir: &Path,
    now: SystemTime,
    fetch: impl FnOnce() -> Result<String, String>,
) -> Result<(Vec<Listing>, Freshness), String> {
    let copy = cached(dir);
    if let Some((list, at)) = &copy
        && now.duration_since(*at).is_ok_and(|age| age < TTL)
    {
        return Ok((list.clone(), Freshness::Fresh));
    }
    // An empty list parses fine but is not an index anyone could use: it must not replace a good copy.
    let downloaded = fetch().and_then(|text| match index::parse(&text)? {
        list if list.is_empty() => Err("herdr's plugin index came empty".to_string()),
        list => Ok((text, list)),
    });
    match downloaded {
        Ok((text, list)) => {
            // Without a copy on disk the explorer still works; it just downloads again next time.
            if let Err(e) = save(dir, &text, now) {
                eprintln!("herdr-market: could not cache the index: {e}");
            }
            Ok((list, Freshness::Fresh))
        }
        Err(e) => match copy {
            Some((list, at)) => Ok((list, Freshness::Stale(at))),
            None => Err(e),
        },
    }
}

/// The copy on disk, whatever its age, and when it was saved: shown while a fresh one downloads.
pub fn cached(dir: &Path) -> Option<(Vec<Listing>, SystemTime)> {
    let saved = fs::read_to_string(dir.join(TIME)).ok()?.trim().parse::<u64>().ok()?;
    let list = index::parse(&fs::read_to_string(dir.join(FILE)).ok()?).ok()?;
    Some((list, UNIX_EPOCH + Duration::from_secs(saved)))
}

fn save(dir: &Path, text: &str, now: SystemTime) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(FILE), text.as_bytes())?;
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    write_atomic(&dir.join(TIME), secs.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::folder;
    use std::time::{Duration, UNIX_EPOCH};

    const INDEX: &str = include_str!("testdata/index.json");

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn first_load_fetches_and_saves() {
        let dir = folder("cache-first");
        let (list, freshness) = load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(freshness, Freshness::Fresh);
    }

    #[test]
    fn fresh_cache_skips_fetch() {
        let dir = folder("cache-fresh");
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let (list, freshness) =
            load_index(&dir, at(1_000_000 + 14 * 60), || panic!("the cache is fresh: no fetch")).unwrap();
        assert_eq!((list.len(), freshness), (5, Freshness::Fresh));
    }

    #[test]
    fn expired_cache_fetches_again() {
        let dir = folder("cache-expired");
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let mut fetched = false;
        load_index(&dir, at(1_000_000 + 15 * 60), || {
            fetched = true;
            Ok(INDEX.to_string())
        })
        .unwrap();
        assert!(fetched);
    }

    #[test]
    fn stale_cache_is_used_when_fetch_fails() {
        let dir = folder("cache-stale");
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let (list, freshness) = load_index(&dir, at(1_000_000 + 3600), || Err("offline".into())).unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(freshness, Freshness::Stale(at(1_000_000)));
    }

    #[test]
    fn cached_copy_regardless_of_age() {
        let dir = folder("cache-any-age");
        assert!(cached(&dir).is_none());
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let (list, saved) = cached(&dir).unwrap();
        assert_eq!((list.len(), saved), (5, at(1_000_000)));
    }

    #[test]
    fn empty_index_does_not_replace_a_good_cache() {
        let dir = folder("cache-empty");
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let (list, freshness) = load_index(&dir, at(1_000_000 + 3600), || Ok(r#"{"plugins": []}"#.into())).unwrap();
        assert_eq!((list.len(), freshness), (5, Freshness::Stale(at(1_000_000))));
        // Without a copy it is an error, not an empty explorer.
        assert!(load_index(&folder("cache-empty-none"), at(1), || Ok(r#"{"plugins": []}"#.into())).is_err());
    }

    #[test]
    fn no_cache_and_no_network_is_an_error() {
        let dir = folder("cache-none");
        let error = load_index(&dir, at(1_000_000), || Err("offline".into())).unwrap_err();
        assert!(error.contains("offline"), "{error}");
    }

    #[test]
    fn a_bad_download_does_not_replace_a_good_cache() {
        let dir = folder("cache-bad");
        load_index(&dir, at(1_000_000), || Ok(INDEX.to_string())).unwrap();
        let (list, freshness) = load_index(&dir, at(1_000_000 + 3600), || Ok("<html>".into())).unwrap();
        assert_eq!((list.len(), freshness), (5, Freshness::Stale(at(1_000_000))));
    }
}
