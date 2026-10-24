//! Bounded, process-local single-flight and cross-session prepared-data reuse.
use super::ResolvedIccPipeline;
use parking_lot::{Condvar, Mutex};
use sha2::{Digest, Sha256};
use spektrafilm_backend::ViewerIccKind;
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

const MAGIC: &[u8; 8] = b"SFICC001";
const HEADER_BYTES: u64 = 84;
const MEMORY_BYTES: usize = 96 * 1024 * 1024;
const DISK_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 16;
const MAX_PREPARATIONS: usize = 2;

type Prepared = Result<Arc<ResolvedIccPipeline>, String>;

#[derive(Default)]
struct Flight {
    result: Mutex<Option<Prepared>>,
    ready: Condvar,
}
impl Flight {
    fn wait(&self) -> Prepared {
        let mut result = self.result.lock();
        while result.is_none() {
            self.ready.wait(&mut result);
        }
        result.as_ref().expect("completed ICC preparation").clone()
    }
}

#[derive(Default)]
struct State {
    entries: VecDeque<Arc<ResolvedIccPipeline>>,
    flights: HashMap<[u8; 32], Arc<Flight>>,
}

struct Cache {
    directory: PathBuf,
    state: Mutex<State>,
    available: Condvar,
}
impl Cache {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            state: Mutex::default(),
            available: Condvar::new(),
        }
    }

    fn get(
        &self,
        identity: [u8; 32],
        prepare: impl FnOnce() -> Result<ResolvedIccPipeline, String>,
    ) -> Result<Arc<ResolvedIccPipeline>, String> {
        let flight = {
            let mut state = self.state.lock();
            loop {
                if let Some(index) = state
                    .entries
                    .iter()
                    .position(|entry| entry.identity == identity)
                {
                    let entry = state.entries.remove(index).expect("cached entry");
                    state.entries.push_front(entry.clone());
                    return Ok(entry);
                }
                if let Some(flight) = state.flights.get(&identity).cloned() {
                    drop(state);
                    return flight.wait();
                }
                if state.flights.len() < MAX_PREPARATIONS {
                    let flight = Arc::new(Flight::default());
                    state.flights.insert(identity, flight.clone());
                    break flight;
                }
                self.available.wait(&mut state);
            }
        };
        // Neither the lookup lock nor the flight-result lock spans IO or LCMS.
        // Bound independent preparations to two maps, while same-key callers
        // share success or failure. Publish a failure on unwind as well so no
        // waiter can remain asleep after a preparation panics.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let path = self.directory.join(filename(identity));
            let prepared = match load(&path, identity) {
                Ok(prepared) => prepared,
                Err(_) => {
                    let prepared = prepare()?;
                    if let Err(error) = self.store(&path, &prepared) {
                        tracing::debug!(%error, "display ICC cache persistence unavailable");
                    }
                    prepared
                }
            };
            Ok(Arc::new(prepared))
        }));
        let result: Prepared = match &outcome {
            Ok(result) => result.clone(),
            Err(_) => Err("ICC preparation panicked".into()),
        };
        {
            let mut state = self.state.lock();
            if let Ok(entry) = &result {
                state.entries.push_front(entry.clone());
                while state.entries.len() > MAX_ENTRIES
                    || state
                        .entries
                        .iter()
                        .map(|entry| entry.words.len() * 4)
                        .sum::<usize>()
                        > MEMORY_BYTES
                {
                    state.entries.pop_back();
                }
            }
            *flight.result.lock() = Some(result.clone());
            state.flights.remove(&identity);
            flight.ready.notify_all();
            self.available.notify_all();
        }
        match outcome {
            Ok(_) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn store(&self, destination: &Path, prepared: &ResolvedIccPipeline) -> io::Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".icc-")
            .tempfile_in(&self.directory)?;
        {
            let mut writer = BufWriter::new(temporary.as_file_mut());
            writer.write_all(MAGIC)?;
            writer.write_all(&prepared.identity)?;
            writer.write_all(&(prepared.kind as u32).to_le_bytes())?;
            writer.write_all(&(prepared.words.len() as u64).to_le_bytes())?;
            writer.write_all(&checksum(&prepared.words))?;
            write_words(&mut writer, &prepared.words)?;
            writer.flush()?;
        }
        temporary.as_file().sync_all()?;
        temporary
            .persist(destination)
            .map_err(|error| error.error)?;
        self.prune()
    }

    fn prune(&self) -> io::Result<()> {
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&self.directory)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                continue;
            }
            let name = item.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".iccdata") else {
                continue;
            };
            if stem.len() != 64 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let metadata = match item.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            entries.push((metadata.modified()?, metadata.len(), item.path()));
        }
        entries.sort_unstable_by_key(|entry| entry.0);
        let mut bytes: u64 = entries.iter().map(|entry| entry.1).sum();
        let mut count = entries.len();
        for (_, size, path) in entries {
            if bytes <= DISK_BYTES && count <= MAX_ENTRIES {
                break;
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            bytes -= size;
            count -= 1;
        }
        Ok(())
    }
}

fn filename(identity: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(72);
    for byte in identity {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 15) as usize] as char);
    }
    name.push_str(".iccdata");
    name
}

fn each_word_chunk(words: &[u32], mut consume: impl FnMut(&[u8])) {
    let mut bytes = [0u8; 4096];
    for chunk in words.chunks(1024) {
        for (word, bytes) in chunk.iter().zip(bytes.as_chunks_mut::<4>().0) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        consume(&bytes[..chunk.len() * 4]);
    }
}
fn checksum(words: &[u32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    each_word_chunk(words, |bytes| hash.update(bytes));
    hash.finalize().into()
}
fn write_words(writer: &mut impl Write, words: &[u32]) -> io::Result<()> {
    let mut result = Ok(());
    each_word_chunk(words, |bytes| {
        if result.is_ok() {
            result = writer.write_all(bytes);
        }
    });
    result
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid prepared ICC cache")
}
fn load(path: &Path, identity: [u8; 32]) -> io::Result<ResolvedIccPipeline> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut header = [0u8; HEADER_BYTES as usize];
    reader.read_exact(&mut header)?;
    if &header[..8] != MAGIC || header[8..40] != identity {
        return Err(invalid());
    }
    let kind = ViewerIccKind::from_id(u32::from_le_bytes(header[40..44].try_into().unwrap()))
        .ok_or_else(invalid)?;
    let count = u64::from_le_bytes(header[44..52].try_into().unwrap());
    if count != kind.word_count() as u64 || size != HEADER_BYTES + count * 4 {
        return Err(invalid());
    }
    let mut words = Vec::with_capacity(kind.word_count());
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 4096];
    while words.len() < kind.word_count() {
        let length = (kind.word_count() - words.len()).min(1024) * 4;
        reader.read_exact(&mut bytes[..length])?;
        hash.update(&bytes[..length]);
        words.extend(
            bytes[..length]
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .map(u32::from_le_bytes),
        );
    }
    let actual: [u8; 32] = hash.finalize().into();
    if actual != header[52..84] {
        return Err(invalid());
    }
    Ok(ResolvedIccPipeline {
        identity,
        kind,
        words,
    })
}

pub(super) fn get(
    identity: [u8; 32],
    prepare: impl FnOnce() -> Result<ResolvedIccPipeline, String>,
) -> Result<Arc<ResolvedIccPipeline>, String> {
    static CACHE: LazyLock<Cache> =
        LazyLock::new(|| Cache::new(crate::state::config_dir().join("cache/display-icc-v1")));
    CACHE.get(identity, prepare)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
