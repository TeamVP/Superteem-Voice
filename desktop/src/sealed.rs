//! Files sealed with the vault's key: meeting audio and journals, a meeting's transcript and what's written
//! about it, the pictures and files kept in notes and books, and kept dictation audio.
//!
//! Every file has its own random key, wrapped in its header by a key derived from the vault key, so one
//! leaked file key opens one file. The body is AES-256-GCM in records, each bound to the file and to its
//! place in it, so records can't be swapped, reordered or cut off unnoticed.
//!
//! Two layouts share the header:
//!
//! - **Whole** files (audio, JSON, attachments) are written in one go, in records of 64 KB, the last one
//!   marked as the last. Any stretch can be read without the rest, which is how audio is played and seeked.
//! - **Growing** files (journals, written as a recording happens) are a run of records of whatever was
//!   written since the last sync, each with its length. Nothing already written is ever sealed again, so
//!   a nonce is never used twice, and a crash loses only the record being written.
//!
//! Reading takes either a sealed file or a plain one, by how it starts: files written before the vault,
//! and on platforms that don't seal yet, are plain. Writing seals when the vault is open and sealed, writes
//! plain where the platform doesn't seal, and refuses while the vault is shut.

use crate::vault;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use stv_core::journal::{self, JournalWriter};
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"STVS";
const VERSION: u8 = 1;
const WHOLE: u8 = 0;
const GROWING: u8 = 1;
const HEADER: usize = 92;
const RECORD: usize = 64 * 1024;
const TAG: usize = 16;

const SHUT: &str = "Your notes, books and meetings are locked. Open them from the home page.";
const FOREIGN: &str = "This was sealed by another vault. Add that vault's recovery words in Settings › Vault to open it.";
const DAMAGED: &str = "This file is damaged and can't be read.";

/// One vault key as files use it: what tells it apart, and the cipher that wraps file keys.
pub struct Opener {
    check: [u8; 8],
    wrap: Aes256Gcm,
}

impl Opener {
    pub fn new(key: &[u8; 32]) -> Opener {
        let key = Zeroizing::new(*key);
        let mut check = [0u8; 8];
        check.copy_from_slice(&vault::derive(&key, b"file-check")[..8]);
        Opener { check, wrap: Aes256Gcm::new_from_slice(&vault::derive(&key, b"files")[..]).expect("a 32-byte key") }
    }
}

/// The keys files can be opened with: the vault's own, which new files are sealed with, then those of
/// vaults brought into it.
pub struct Ring(pub Vec<Opener>);

#[derive(Clone)]
#[cfg_attr(test, allow(dead_code))]
enum Mode {
    /// The vault hasn't been asked yet.
    Unset,
    /// This platform doesn't seal: files are written as they always were.
    Plain,
    Shut,
    Sealed(Arc<Ring>),
}

#[cfg_attr(test, allow(dead_code))]
static MODE: RwLock<Mode> = RwLock::new(Mode::Unset);

#[cfg(test)]
thread_local! {
    static TEST: std::cell::RefCell<Option<Arc<Ring>>> = const { std::cell::RefCell::new(None) };
}

/// Seals what this test writes from here on, with a key of its own. Tests otherwise write plain files,
/// whatever the platform, so they can read what they wrote.
#[cfg(test)]
pub fn seal_in_this_test(key: [u8; 32]) {
    TEST.with(|t| *t.borrow_mut() = Some(Arc::new(Ring(vec![Opener::new(&key)]))));
}

/// Told by the vault as it opens and shuts.
pub fn set(ring: Option<Ring>) {
    let mode = match ring {
        Some(ring) => Mode::Sealed(Arc::new(ring)),
        None if vault::SEALED => Mode::Shut,
        None => Mode::Plain,
    };
    *MODE.write().unwrap_or_else(|p| p.into_inner()) = mode;
}

#[cfg(test)]
fn mode() -> Mode {
    TEST.with(|t| t.borrow().clone()).map_or(Mode::Plain, Mode::Sealed)
}

#[cfg(not(test))]
fn mode() -> Mode {
    let read = || MODE.read().unwrap_or_else(|p| p.into_inner()).clone();
    if matches!(read(), Mode::Unset) {
        // Opening the vault tells this module what it found.
        vault::touch();
    }
    read()
}

/// Whether files are sealed here and the vault is shut, so nothing can be read or written.
pub fn shut() -> bool {
    matches!(mode(), Mode::Shut)
}

/// Fails while the vault is shut, before something is started that would only fail later.
pub fn ready() -> Result<(), String> {
    if shut() { Err(SHUT.into()) } else { Ok(()) }
}

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn nonce(index: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&index.to_le_bytes());
    nonce
}

/// What a record is bound to: its file, its place, and whether it is the last (2 for a growing file,
/// which has no last record).
fn bound(id: &[u8; 16], index: u64, kind: u8) -> [u8; 25] {
    let mut aad = [0u8; 25];
    aad[..16].copy_from_slice(id);
    aad[16..24].copy_from_slice(&index.to_le_bytes());
    aad[24] = kind;
    aad
}

/// A sealed file's header, opened: its layout, and the cipher for its records.
struct Opened {
    layout: u8,
    id: [u8; 16],
    cipher: Aes256Gcm,
}

fn is_header(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..4] == MAGIC
}

/// Makes a header for a new file, and the cipher for its records.
fn new_header(ring: &Ring, layout: u8) -> Result<([u8; HEADER], Opened), String> {
    let own = ring.0.first().ok_or(SHUT)?;
    let mut random = [0u8; 16 + 12 + 32];
    getrandom::getrandom(&mut random).map_err(e)?;
    let file_key = Zeroizing::new(<[u8; 32]>::try_from(&random[28..]).unwrap());
    let mut header = [0u8; HEADER];
    header[..4].copy_from_slice(MAGIC);
    header[4] = VERSION;
    header[5] = layout;
    header[8..24].copy_from_slice(&random[..16]);
    header[24..32].copy_from_slice(&own.check);
    header[32..44].copy_from_slice(&random[16..28]);
    let wrapped = own
        .wrap
        .encrypt(Nonce::from_slice(&random[16..28]), Payload { msg: &file_key[..], aad: &header[..32] })
        .map_err(|_| "The file couldn't be sealed.".to_string())?;
    header[44..].copy_from_slice(&wrapped);
    random[28..].fill(0);
    let opened = Opened { layout, id: header[8..24].try_into().unwrap(), cipher: Aes256Gcm::new_from_slice(&file_key[..]).map_err(e)? };
    Ok((header, opened))
}

fn open_header(header: &[u8]) -> Result<Opened, String> {
    if header.len() < HEADER || !is_header(header) || header[4] != VERSION || header[5] > GROWING {
        return Err(DAMAGED.into());
    }
    let ring = match mode() {
        Mode::Sealed(ring) => ring,
        Mode::Shut | Mode::Unset => return Err(SHUT.into()),
        // A sealed file where nothing seals: one copied from a computer that does.
        Mode::Plain => return Err(FOREIGN.into()),
    };
    let opener = ring.0.iter().find(|o| o.check == header[24..32]).ok_or(FOREIGN)?;
    let file_key = Zeroizing::new(
        opener.wrap.decrypt(Nonce::from_slice(&header[32..44]), Payload { msg: &header[44..HEADER], aad: &header[..32] }).map_err(|_| DAMAGED.to_string())?,
    );
    Ok(Opened { layout: header[5], id: header[8..24].try_into().unwrap(), cipher: Aes256Gcm::new_from_slice(&file_key[..]).map_err(e)? })
}

impl Opened {
    fn seal(&self, index: u64, kind: u8, plain: &[u8]) -> Result<Vec<u8>, String> {
        self.cipher.encrypt(Nonce::from_slice(&nonce(index)), Payload { msg: plain, aad: &bound(&self.id, index, kind) }).map_err(|_| "The file couldn't be sealed.".to_string())
    }

    fn open(&self, index: u64, kind: u8, sealed: &[u8]) -> Option<Vec<u8>> {
        self.cipher.decrypt(Nonce::from_slice(&nonce(index)), Payload { msg: sealed, aad: &bound(&self.id, index, kind) }).ok()
    }
}

/// The bytes of a whole file, sealed.
fn seal_whole(ring: &Ring, plain: &[u8]) -> Result<Vec<u8>, String> {
    let (header, opened) = new_header(ring, WHOLE)?;
    let records = plain.len().div_ceil(RECORD).max(1);
    let mut out = Vec::with_capacity(HEADER + plain.len() + records * TAG);
    out.extend_from_slice(&header);
    for index in 0..records {
        let chunk = &plain[index * RECORD..plain.len().min((index + 1) * RECORD)];
        out.extend(opened.seal(index as u64, (index + 1 == records) as u8, chunk)?);
    }
    Ok(out)
}

/// How long a whole file's contents are, from the length of its sealed body.
fn whole_len(body: u64) -> Option<u64> {
    let full = RECORD as u64 + TAG as u64;
    match (body / full, body % full) {
        (0, 0) => None,
        (records, 0) => Some(records * RECORD as u64),
        (_, rest) if rest < TAG as u64 => None,
        (records, rest) => Some(records * RECORD as u64 + rest - TAG as u64),
    }
}

fn open_whole(opened: &Opened, body: &[u8]) -> Result<Vec<u8>, String> {
    let len = whole_len(body.len() as u64).ok_or(DAMAGED)? as usize;
    let records = len.div_ceil(RECORD).max(1);
    let mut out = Vec::with_capacity(len);
    for (index, sealed) in body.chunks(RECORD + TAG).enumerate() {
        out.extend(opened.open(index as u64, (index + 1 == records) as u8, sealed).ok_or(DAMAGED)?);
    }
    Ok(out)
}

/// The records of a growing file from `body` on, the first being record `index`: what they held, and how
/// many bytes of `body` they took. A record cut short or still being written is left.
fn open_growing(opened: &Opened, body: &[u8], mut index: u64) -> (Vec<u8>, usize, u64) {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(head) = body.get(pos..pos + 4) {
        let len = u32::from_le_bytes(head.try_into().unwrap()) as usize;
        if !(TAG..=RECORD + TAG).contains(&len) {
            break;
        }
        let Some(plain) = body.get(pos + 4..pos + 4 + len).and_then(|sealed| opened.open(index, 2, sealed)) else { break };
        out.extend(plain);
        pos += 4 + len;
        index += 1;
    }
    (out, pos, index)
}

/// Whether the file is sealed, by how it starts.
pub fn is_sealed(path: &Path) -> bool {
    let mut head = [0u8; 4];
    File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && is_header(&head)
}

/// What a file holds, sealed or plain.
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !is_header(&data) {
        return Ok(data);
    }
    let opened = open_header(&data)?;
    match opened.layout {
        WHOLE => open_whole(&opened, &data[HEADER..]),
        _ => Ok(open_growing(&opened, &data[HEADER..], 0).0),
    }
}

/// The file's name with `.part` after it: where it's written before taking the file's place.
fn partial(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Writes a file whole, sealed where files are. It's written beside the target and renamed, so a crash
/// mid-save never leaves half a file.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let sealed;
    let out = match mode() {
        Mode::Plain => bytes,
        Mode::Shut | Mode::Unset => return Err(SHUT.into()),
        Mode::Sealed(ring) => {
            // A file of another vault's is never written over: until that vault is brought in it can't be
            // read here, and what would replace it is whatever was shown in its place.
            let mut head = [0u8; HEADER];
            if File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && is_header(&head) && !ring.0.iter().any(|o| o.check == head[24..32]) {
                return Err(FOREIGN.into());
            }
            sealed = seal_whole(&ring, bytes)?;
            &sealed[..]
        }
    };
    let part = partial(path);
    std::fs::write(&part, out).map_err(e)?;
    std::fs::rename(&part, path).map_err(e)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    write(path, serde_json::to_string_pretty(value).map_err(e)?.as_bytes())
}

pub fn write_wav(path: &Path, pcm: &[i16], rate: u32) -> Result<(), String> {
    write(path, &stv_core::export::wav_bytes(pcm, rate))
}

/// Writes what a file holds somewhere outside the app, in the clear: an export, which is the way out.
pub fn copy_plain(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::write(to, read(from)?).map_err(|e| format!("{}: {e}", to.display()))
}

// ---------------------------------------------------------------------------------------------------------
// Reading a stretch of a file, for playing audio

/// A file open for reading any stretch of it, without the rest.
pub struct Reader {
    file: File,
    /// None for a plain file.
    opened: Option<Opened>,
    len: u64,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Reader, String> {
        let mut file = File::open(path).map_err(e)?;
        let size = file.metadata().map_err(e)?.len();
        let mut head = [0u8; HEADER];
        if size < 4 || file.read_exact(&mut head[..4]).is_err() || !is_header(&head) {
            return Ok(Reader { file, opened: None, len: size });
        }
        file.read_exact(&mut head[4..]).map_err(|_| DAMAGED.to_string())?;
        let opened = open_header(&head)?;
        if opened.layout != WHOLE {
            return Err("A recording still being made can't be played.".into());
        }
        let len = whole_len(size - HEADER as u64).ok_or(DAMAGED)?;
        Ok(Reader { file, opened: Some(opened), len })
    }

    /// How long the file's contents are.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Up to `count` bytes from `start`.
    pub fn read(&mut self, start: u64, count: usize) -> Result<Vec<u8>, String> {
        let end = self.len.min(start.saturating_add(count as u64));
        if start >= end {
            return Ok(Vec::new());
        }
        let Some(opened) = &self.opened else {
            let mut out = vec![0u8; (end - start) as usize];
            self.file.seek(SeekFrom::Start(start)).map_err(e)?;
            self.file.read_exact(&mut out).map_err(e)?;
            return Ok(out);
        };
        let last = self.len.div_ceil(RECORD as u64).max(1) - 1;
        let mut out = Vec::with_capacity((end - start) as usize);
        let mut sealed = vec![0u8; RECORD + TAG];
        for index in start / RECORD as u64..=(end - 1) / RECORD as u64 {
            let at = index * RECORD as u64;
            let size = (self.len - at).min(RECORD as u64) as usize + TAG;
            self.file.seek(SeekFrom::Start(HEADER as u64 + index * (RECORD + TAG) as u64)).map_err(e)?;
            self.file.read_exact(&mut sealed[..size]).map_err(|_| DAMAGED.to_string())?;
            let plain = opened.open(index, (index == last) as u8, &sealed[..size]).ok_or(DAMAGED)?;
            let from = start.saturating_sub(at) as usize;
            let to = ((end - at) as usize).min(plain.len());
            out.extend_from_slice(&plain[from.min(to)..to]);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Journals: files that grow as a recording happens

/// A journal's sink that seals what it's given: a record for each sync, or each 64 KB if that comes first.
struct Growing {
    file: File,
    opened: Opened,
    index: u64,
    waiting: Vec<u8>,
}

impl Growing {
    fn seal(&mut self, count: usize) -> io::Result<()> {
        let sealed = self.opened.seal(self.index, 2, &self.waiting[..count]).map_err(io::Error::other)?;
        // One write, so a record is in the file whole or not at all as far as this process goes.
        let mut record = Vec::with_capacity(4 + sealed.len());
        record.extend_from_slice(&(sealed.len() as u32).to_le_bytes());
        record.extend_from_slice(&sealed);
        self.file.write_all(&record)?;
        self.index += 1;
        self.waiting.drain(..count);
        Ok(())
    }
}

impl Write for Growing {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.waiting.extend_from_slice(buf);
        while self.waiting.len() >= RECORD {
            self.seal(RECORD)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.waiting.is_empty() {
            self.seal(self.waiting.len())?;
        }
        self.file.flush()
    }
}

impl journal::Sink for Growing {
    fn sync(&mut self) -> io::Result<()> {
        self.flush()?;
        self.file.sync_data()
    }
}

impl Drop for Growing {
    fn drop(&mut self) {
        // What was written since the last sync is still sealed and kept when the recording ends.
        let _ = self.flush();
        self.waiting.fill(0);
    }
}

/// Starts a journal, sealed where files are.
pub fn journal_writer(path: &Path) -> Result<JournalWriter, String> {
    match mode() {
        Mode::Plain => JournalWriter::create(path).map_err(e),
        Mode::Shut | Mode::Unset => Err(SHUT.into()),
        Mode::Sealed(ring) => {
            let (header, opened) = new_header(&ring, GROWING)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(e)?;
            file.write_all(&header).map_err(e)?;
            JournalWriter::to(Box::new(Growing { file, opened, index: 0, waiting: Vec::new() })).map_err(e)
        }
    }
}

/// A journal's audio, aligned, from a sealed or plain journal.
pub fn load_source(path: &Path, rate: u32) -> Result<stv_core::pipeline::SourceTrack, String> {
    let recovered = journal::parse(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    stv_core::pipeline::source_from(recovered, rate).map_err(|e| format!("{}: {e}", path.display()))
}

/// Follows a journal that is still being written, sealed or plain: each call gives the records written
/// since the last.
#[derive(Default)]
pub struct Tail {
    /// How far into the file has been read.
    at: u64,
    /// The sealed file's header, opened, and the number of its next record.
    opened: Option<(Opened, u64)>,
    /// Journal bytes read but not yet whole records.
    waiting: Vec<u8>,
    started: bool,
}

impl Tail {
    /// Whether anything has been read yet.
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn more(&mut self, path: &Path) -> Result<Vec<journal::Record>, String> {
        let mut file = File::open(path).map_err(e)?;
        if self.at == 0 {
            let mut head = [0u8; HEADER];
            let got = file.read(&mut head).map_err(e)?;
            if is_header(&head[..got]) {
                if got < HEADER {
                    return Ok(Vec::new());
                }
                self.opened = Some((open_header(&head)?, 0));
                self.at = HEADER as u64;
            }
        }
        file.seek(SeekFrom::Start(self.at)).map_err(e)?;
        let mut fresh = Vec::new();
        file.read_to_end(&mut fresh).map_err(e)?;
        match &mut self.opened {
            Some((opened, index)) => {
                let (plain, used, next) = open_growing(opened, &fresh, *index);
                *index = next;
                self.at += used as u64;
                self.waiting.extend(plain);
            }
            None => {
                self.at += fresh.len() as u64;
                self.waiting.extend(fresh);
            }
        }
        if !self.started {
            if self.waiting.len() < journal::HEADER_BYTES {
                return Ok(Vec::new());
            }
            if !journal::is_journal(&self.waiting) {
                return Err("not a journal file".into());
            }
            self.waiting.drain(..journal::HEADER_BYTES);
            self.started = true;
        }
        let (records, used) = journal::parse_records(&self.waiting);
        self.waiting.drain(..used);
        Ok(records)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Sealing what was written before the vault

/// The readable copies of a meeting that used to be written into its folder for opening outside the app.
/// Sealed meetings don't have them; Export is the way out.
pub const READABLE_COPIES: [&str; 4] = ["transcript.txt", "transcript.md", "transcript.srt", "summary.md"];

static LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many files written before the vault are still to be sealed.
pub fn left() -> usize {
    LEFT.load(std::sync::atomic::Ordering::Relaxed)
}

/// The files under the app's folders that hold what people said or wrote, and so are sealed.
fn sealable() -> Vec<PathBuf> {
    let files = |dir: PathBuf| std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect::<Vec<_>>();
    let has = |path: &Path, kinds: &[&str]| path.extension().and_then(|x| x.to_str()).is_some_and(|x| kinds.contains(&x));
    let mut out = Vec::new();
    for dir in files(crate::library::root()).into_iter().filter(|p| p.is_dir()) {
        if !dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("session-") && !n.ends_with(".part")) {
            continue;
        }
        out.extend(files(dir.clone()).into_iter().filter(|p| p.is_file() && (has(p, &["wav", "stvj", "json"]) || crate::imports::is_track_file(p))));
        out.extend(files(dir.join("translations")).into_iter().filter(|p| has(p, &["json"])));
    }
    let data = crate::library::data_dir();
    for dir in files(data.join("notes")).into_iter().filter(|p| p.is_dir()) {
        match dir.file_name().and_then(|n| n.to_str()) {
            Some("audio") => out.extend(files(dir).into_iter().filter(|p| has(p, &["wav"]))),
            Some(name) if name.ends_with(".files") => out.extend(files(dir).into_iter().filter(|p| p.is_file())),
            _ => {}
        }
    }
    // A book's own files (its cover, pictures, scans and their register), in folders of their own.
    fn tree(dir: &Path, out: &mut Vec<PathBuf>) {
        for path in std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()) {
            if path.is_dir() {
                tree(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    for dir in files(data.join("books")).into_iter().filter(|p| p.is_dir() && p.to_string_lossy().ends_with(".files")) {
        tree(&dir, &mut out);
    }
    // Recordings made to learn a voice or a phrase.
    for dir in files(data).into_iter().filter(|p| p.is_dir()) {
        if dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("enrol-") || n.starts_with("phrase-")) {
            out.extend(files(dir).into_iter().filter(|p| has(p, &["stvj", "wav"])));
        }
    }
    out.retain(|p| !p.to_string_lossy().ends_with(".part"));
    out
}

/// Seals one plain file in place: sealed beside it, read back and compared, then put in its place. It goes a
/// record at a time, so a meeting's hours-long track never has to fit in memory.
fn seal_in_place(path: &Path) -> Result<(), String> {
    let Mode::Sealed(ring) = mode() else { return Err(SHUT.into()) };
    let mut src = File::open(path).map_err(e)?;
    let len = src.metadata().map_err(e)?.len();
    let mut head = [0u8; 4];
    if len >= 4 {
        src.read_exact(&mut head).map_err(e)?;
        if is_header(&head) {
            return Ok(());
        }
        src.seek(SeekFrom::Start(0)).map_err(e)?;
    }
    let part = partial(path);
    let written = (|| -> Result<(), String> {
        let (header, opened) = new_header(&ring, WHOLE)?;
        let mut out = io::BufWriter::new(File::create(&part).map_err(e)?);
        out.write_all(&header).map_err(e)?;
        let records = len.div_ceil(RECORD as u64).max(1);
        let mut buf = Zeroizing::new(vec![0u8; RECORD]);
        for index in 0..records {
            let size = (len - index * RECORD as u64).min(RECORD as u64) as usize;
            src.read_exact(&mut buf[..size]).map_err(e)?;
            out.write_all(&opened.seal(index, (index + 1 == records) as u8, &buf[..size])?).map_err(e)?;
        }
        out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(e)
    })();
    // Read back and compared with the original, record by record, before it takes the original's place.
    let same = written.and_then(|_| {
        let mut back = Reader::open(&part)?;
        if back.len() != len {
            return Ok(false);
        }
        src.seek(SeekFrom::Start(0)).map_err(e)?;
        let mut want = Zeroizing::new(vec![0u8; RECORD]);
        let mut at = 0;
        while at < len {
            let size = (len - at).min(RECORD as u64) as usize;
            src.read_exact(&mut want[..size]).map_err(e)?;
            if back.read(at, size)?[..] != want[..size] {
                return Ok(false);
            }
            at += size as u64;
        }
        Ok(true)
    });
    drop(src);
    match same {
        Ok(true) => std::fs::rename(&part, path).map_err(e),
        Ok(false) => {
            let _ = std::fs::remove_file(&part);
            Err("it didn't read back the same".into())
        }
        Err(why) => {
            let _ = std::fs::remove_file(&part);
            Err(why)
        }
    }
}

/// Seals the files written before the vault, and removes the readable copies of meetings. Run in the
/// background as the app starts; whatever it doesn't get to (a file in use, the app closing) waits for
/// next time. A journal being recorded now is already sealed, so nothing in use is touched.
pub fn seal_existing() {
    // One pass at a time: two at once would seal the same file side by side and trip over each other's
    // renames. A pass asked for while one runs waits, then finds what the first left.
    static RUNNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    if !matches!(mode(), Mode::Sealed(_)) {
        return;
    }
    let todo: Vec<PathBuf> = sealable().into_iter().filter(|p| !is_sealed(p)).collect();
    LEFT.store(todo.len(), std::sync::atomic::Ordering::Relaxed);
    for path in todo {
        if let Err(why) = seal_in_place(&path) {
            eprintln!("{} wasn't sealed: {why}", path.display());
        }
        LEFT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
    for dir in std::fs::read_dir(crate::library::root()).into_iter().flatten().flatten().map(|e| e.path()) {
        // Only from meetings whose transcript is sealed, so the readable copy is never the only one.
        if dir.is_dir() && is_sealed(&dir.join("transcript.json")) {
            for name in READABLE_COPIES {
                let _ = std::fs::remove_file(dir.join(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stv-sealed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn whole_files_read_back_at_every_awkward_length() {
        let dir = folder("whole");
        seal_in_this_test([3; 32]);
        for len in [0, 1, RECORD - 1, RECORD, RECORD + 1, 3 * RECORD, 3 * RECORD + 77] {
            let (path, plain) = (dir.join(format!("f{len}")), pattern(len));
            write(&path, &plain).unwrap();
            let raw = std::fs::read(&path).unwrap();
            assert!(is_sealed(&path) && raw.len() == HEADER + len + len.div_ceil(RECORD).max(1) * TAG, "length {len}");
            assert!(len < 64 || !raw.windows(32).any(|w| w == &plain[..32]), "the contents aren't in the file as they are");
            assert_eq!(read(&path).unwrap(), plain, "length {len}");
            let mut reader = Reader::open(&path).unwrap();
            assert_eq!(reader.len(), len as u64);
            // Any stretch, across record edges and past the end.
            for (start, count) in [(0, 10), (RECORD - 3, 7), (len / 2, RECORD + 5), (len.saturating_sub(4), 100), (len, 5), (len + 9, 5)] {
                let want = &plain[start.min(len)..(start + count).min(len)];
                assert_eq!(reader.read(start as u64, count).unwrap(), want, "length {len}, {count} from {start}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tampering_cutting_and_swapping_are_caught() {
        let dir = folder("tamper");
        seal_in_this_test([4; 32]);
        let (a, b) = (dir.join("a"), dir.join("b"));
        write(&a, &pattern(2 * RECORD + 500)).unwrap();
        write(&b, &pattern(2 * RECORD + 500)).unwrap();
        let (raw_a, raw_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        assert_ne!(raw_a[HEADER..HEADER + 64], raw_b[HEADER..HEADER + 64], "the same contents seal differently each time");
        let record = RECORD + TAG;
        let try_with = |change: &dyn Fn(&mut Vec<u8>)| {
            let mut raw = raw_a.clone();
            change(&mut raw);
            std::fs::write(&a, raw).unwrap();
            read(&a)
        };
        assert!(try_with(&|raw| raw[HEADER + 100] ^= 1).is_err(), "a changed byte");
        assert!(try_with(&|raw| raw[50] ^= 1).is_err(), "a changed header");
        assert!(try_with(&|raw| raw.truncate(HEADER + 2 * record)).is_err(), "the last record cut off");
        assert!(try_with(&|raw| raw.truncate(HEADER + record + 40)).is_err(), "cut mid-record");
        assert!(try_with(&|raw| raw[HEADER..HEADER + 2 * record].rotate_left(record)).is_err(), "two records swapped");
        assert!(try_with(&|raw| raw[HEADER..HEADER + record].copy_from_slice(&raw_b[HEADER..HEADER + record])).is_err(), "a record from another file");
        assert!(try_with(&|_| {}).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plain_files_are_read_as_they_are_and_another_vaults_are_left_alone() {
        let dir = folder("plain");
        let path = dir.join("meta.json");
        // Nothing seals in a test until it asks.
        write_json(&path, &serde_json::json!({ "title": "Before the vault" })).unwrap();
        assert!(!is_sealed(&path));
        seal_in_this_test([5; 32]);
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap()["title"], "Before the vault");
        seal_in_place(&path).unwrap();
        assert!(is_sealed(&path) && !partial(&path).exists());
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap()["title"], "Before the vault");

        // Under another vault's key the file can't be read, and isn't written over.
        seal_in_this_test([6; 32]);
        assert_eq!(read(&path).unwrap_err(), FOREIGN);
        assert_eq!(write(&path, b"{}").unwrap_err(), FOREIGN);
        // With that vault brought in, it opens again.
        TEST.with(|t| *t.borrow_mut() = Some(Arc::new(Ring(vec![Opener::new(&[6; 32]), Opener::new(&[5; 32])]))));
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap()["title"], "Before the vault");
        write(&path, b"{}").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_long_plain_recording_is_sealed_in_place_a_record_at_a_time() {
        let dir = folder("in-place");
        seal_in_this_test([8; 32]);
        for len in [0, 3, RECORD, 3 * RECORD + 77] {
            let (path, plain) = (dir.join(format!("input{len}.wav")), pattern(len));
            std::fs::write(&path, &plain).unwrap();
            seal_in_place(&path).unwrap();
            assert!(is_sealed(&path) && !partial(&path).exists(), "length {len}");
            assert_eq!(read(&path).unwrap(), plain, "length {len}");
            let raw = std::fs::read(&path).unwrap();
            assert_eq!(raw.len(), HEADER + len + len.div_ceil(RECORD).max(1) * TAG, "the same layout as a file sealed whole");
            // Sealing again leaves it as it is.
            seal_in_place(&path).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), raw);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_sealed_journal_is_followed_as_it_grows_and_survives_a_crash() {
        let dir = folder("journal");
        for sealing in [false, true] {
            if sealing {
                seal_in_this_test([7; 32]);
            }
            let path = dir.join(format!("input-{sealing}.stvj"));
            let mut writer = journal_writer(&path).unwrap();
            let mut tail = Tail::default();
            assert!(tail.more(&path).unwrap().is_empty() && tail.started());
            let frame = |n: u8| vec![n; 700];
            for n in 0..5u8 {
                writer.append(n as u64, &frame(n)).unwrap();
            }
            assert!(tail.more(&path).unwrap().is_empty(), "nothing is in the file until it's synced");
            writer.sync().unwrap();
            let got = tail.more(&path).unwrap();
            assert_eq!(got.iter().map(|r| r.arrival_us).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
            // More than one record's worth between syncs.
            for n in 5..205u8 {
                writer.append(n as u64, &frame(n)).unwrap();
            }
            writer.sync().unwrap();
            assert_eq!(tail.more(&path).unwrap().len(), 200);
            writer.append(999, &frame(9)).unwrap();
            drop(writer);
            assert_eq!(tail.more(&path).unwrap().len(), 1, "what was written last is kept when the recording ends");
            assert_eq!(is_sealed(&path), sealing);
            let all = journal::parse(&read(&path).unwrap()).unwrap();
            assert_eq!((all.records.len(), all.had_bad_tail), (206, false));
            assert_eq!(all.records[205], journal::Record { arrival_us: 999, frame: frame(9) });

            // A crash mid-write: the torn end is dropped and everything before it is read.
            let mut raw = std::fs::read(&path).unwrap();
            raw.truncate(raw.len() - 300);
            std::fs::write(&path, raw).unwrap();
            let kept = journal::parse(&read(&path).unwrap()).unwrap().records.len();
            assert_eq!(kept, 205, "sealing: {sealing}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
