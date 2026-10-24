use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

fn prepared(identity: [u8; 32], value: u32) -> ResolvedIccPipeline {
    ResolvedIccPipeline {
        identity,
        kind: ViewerIccKind::Curves,
        words: vec![value; ViewerIccKind::Curves.word_count()],
    }
}

#[test]
fn restart_reuses_checked_preparation_and_corruption_reprepares() {
    let directory = tempfile::tempdir().unwrap();
    let identity = [7; 32];
    let path = directory.path().join(filename(identity));
    let cache = Cache::new(directory.path().to_owned());
    let first = cache
        .get(identity, || Ok(prepared(identity, 0x0123_4567)))
        .unwrap();
    drop(cache);
    let restarted = Cache::new(directory.path().to_owned());
    let second = restarted
        .get(identity, || panic!("valid disk hit must not run LCMS"))
        .unwrap();
    assert_eq!(first.words, second.words);
    assert_eq!(first.kind, second.kind);
    assert!(!Arc::ptr_eq(&first, &second));
    drop(restarted);

    let valid = std::fs::read(&path).unwrap();
    // Cover the format, key, kind, declared length, checksum, payload,
    // truncation and trailing-data boundaries independently.
    for offset in [
        0,
        8,
        40,
        44,
        52,
        HEADER_BYTES as usize,
        valid.len(),
        valid.len() + 1,
    ] {
        let mut corrupt = valid.clone();
        if offset == valid.len() {
            corrupt.pop();
        } else if offset > valid.len() {
            corrupt.push(0);
        } else {
            corrupt[offset] ^= 0xff;
        }
        std::fs::write(&path, corrupt).unwrap();
        let cache = Cache::new(directory.path().to_owned());
        let calls = AtomicUsize::new(0);
        let entry = cache
            .get(identity, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(prepared(identity, 0x7654_3210))
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "offset {offset}");
        assert_eq!(
            entry.words,
            vec![0x7654_3210; ViewerIccKind::Curves.word_count()]
        );
        assert_eq!(load(&path, identity).unwrap().words, entry.words);
    }
}

#[test]
fn concurrent_same_key_shares_one_preparation_and_arc() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path().to_owned());
    let calls = AtomicUsize::new(0);
    let start = std::sync::Barrier::new(8);
    let entries = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    cache
                        .get([3; 32], || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(prepared([3; 32], 42))
                        })
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(entries.iter().all(|entry| Arc::ptr_eq(entry, &entries[0])));
}

#[test]
fn unrelated_misses_and_memory_hits_do_not_wait_for_preparation() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path().to_owned());
    cache.get([1; 32], || Ok(prepared([1; 32], 1))).unwrap();
    std::thread::scope(|scope| {
        let cache = &cache;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_a_tx, release_a_rx) = mpsc::channel();
        let (release_b_tx, release_b_rx) = mpsc::channel();
        let tx = started_tx.clone();
        let a = scope.spawn(move || {
            cache.get([2; 32], move || {
                tx.send(2).unwrap();
                release_a_rx.recv().unwrap();
                Ok(prepared([2; 32], 2))
            })
        });
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        let b = scope.spawn(move || {
            cache.get([3; 32], move || {
                started_tx.send(3).unwrap();
                release_b_rx.recv().unwrap();
                Ok(prepared([3; 32], 3))
            })
        });
        let (hit_tx, hit_rx) = mpsc::channel();
        let hit = scope.spawn(move || {
            assert!(
                hit_tx
                    .send(cache.get([1; 32], || panic!("memory hit")))
                    .is_ok()
            );
        });
        let second_started = started_rx.recv_timeout(Duration::from_secs(5));
        let cached = hit_rx.recv_timeout(Duration::from_secs(5));
        // Release workers before asserting so a failed assertion cannot hang
        // scoped thread teardown.
        release_a_tx.send(()).unwrap();
        release_b_tx.send(()).unwrap();
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        hit.join().unwrap();
        assert_eq!(
            second_started.unwrap(),
            3,
            "different keys must prepare concurrently"
        );
        assert_eq!(
            cached.unwrap().unwrap().words[0],
            1,
            "cache hits must not wait for LCMS"
        );
    });
}

#[test]
fn preparation_failure_releases_flight_and_can_retry() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path().to_owned());
    assert_eq!(
        cache
            .get([4; 32], || Err("invalid profile".into()))
            .err()
            .unwrap(),
        "invalid profile"
    );
    assert!(cache.state.lock().flights.is_empty());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = cache.get([4; 32], || panic!("LCMS preparation panic"));
        }))
        .is_err()
    );
    assert!(cache.state.lock().flights.is_empty());
    assert_eq!(
        cache
            .get([4; 32], || Ok(prepared([4; 32], 4)))
            .unwrap()
            .words[0],
        4
    );
}

#[test]
fn memory_lru_and_disk_count_are_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path().to_owned());
    for key in 0..MAX_ENTRIES as u8 {
        cache
            .get([key; 32], || Ok(prepared([key; 32], u32::from(key))))
            .unwrap();
    }
    cache.get([0; 32], || panic!("hot entry")).unwrap();
    cache.get([99; 32], || Ok(prepared([99; 32], 99))).unwrap();
    let state = cache.state.lock();
    assert_eq!(state.entries.len(), MAX_ENTRIES);
    assert!(state.entries.iter().any(|entry| entry.identity == [0; 32]));
    assert!(!state.entries.iter().any(|entry| entry.identity == [1; 32]));
    drop(state);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        MAX_ENTRIES
    );
}

#[test]
fn memory_bytes_and_disk_bytes_are_bounded_and_persistence_is_optional() {
    let directory = tempfile::tempdir().unwrap();
    // A file cannot be a cache directory. Preparation must still work, without
    // writing three exhaustive maps merely to test the memory limit.
    let unavailable = directory.path().join("not-a-directory");
    File::create(&unavailable).unwrap();
    let cache = Cache::new(unavailable);
    for key in 0..3 {
        cache
            .get([key; 32], || {
                Ok(ResolvedIccPipeline {
                    identity: [key; 32],
                    kind: ViewerIccKind::ExactRgb8,
                    words: vec![0; ViewerIccKind::ExactRgb8.word_count()],
                })
            })
            .unwrap();
    }
    let state = cache.state.lock();
    assert_eq!(state.entries.len(), 2);
    assert_eq!(
        state
            .entries
            .iter()
            .map(|entry| entry.words.len() * 4)
            .sum::<usize>(),
        MEMORY_BYTES
    );
    drop(state);
    let cache = Cache::new(directory.path().join("disk"));
    std::fs::create_dir(&cache.directory).unwrap();
    for key in 0..3 {
        File::create(cache.directory.join(filename([key; 32])))
            .unwrap()
            .set_len(DISK_BYTES / 2)
            .unwrap();
    }
    cache.prune().unwrap();
    let files: Vec<_> = std::fs::read_dir(&cache.directory)
        .unwrap()
        .map(|item| item.unwrap())
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!(
        files
            .iter()
            .map(|file| file.metadata().unwrap().len())
            .sum::<u64>(),
        DISK_BYTES
    );
}
