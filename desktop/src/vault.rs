//! The vault: one database that holds the notes and everything written about them (versions, chats,
//! analyses, translations, bookmarks), and the books (with where reading aloud stopped).
//!
//! On Windows the database is SQLCipher, encrypted with a key derived from the vault key: 32 random bytes
//! made when the vault is. Three things can release the vault key, and any one is enough:
//!
//! - **This Windows account** (DPAPI), without asking. This is how the vault opens day to day.
//! - **The recovery words**: the vault key itself written as 24 words. They need nothing but the database
//!   file, so they still work on another computer, after Windows is reinstalled, and when `keys.json` is lost.
//! - **A vault password**, if the person set one, with a hint they wrote for themselves.
//!
//! `keys.json` beside the database holds the wrapped copies of the key and is not secret. The database is
//! copied (sealed with the same key) into `snapshots/` as it changes, and a damaged one is set aside, never
//! deleted, when a copy takes its place.
//!
//! On the other platforms the same database is plain SQLite for now, as their files were plain before.

use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

/// Whether the database is encrypted on this platform.
pub const SEALED: bool = cfg!(windows);

/// What a command fails with while the vault is shut; the page shows the way back in.
pub const LOCKED: &str = "Your notes, books and meetings are locked. Open them from the home page.";

type Key = Zeroizing<[u8; 32]>;

const DB: &str = "vault.db";
const KEYS: &str = "keys.json";
const SNAPSHOTS: &str = "snapshots";
/// A database up to this size is checked page by page each time it's opened.
const CHECK_UP_TO: u64 = 64 * 1024 * 1024;
/// A changed vault is copied once it's this long since the last copy.
const SNAPSHOT_EVERY_S: u64 = 3600;

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn now() -> u64 {
    crate::library::now()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(text: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| "The vault's key file is damaged.".to_string())
}

fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).map_err(|e| format!("This device gave no random numbers: {e}"))?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// Keys

/// What is derived from the vault key, so the key itself is never handed to anything.
pub(crate) fn derive(key: &Key, purpose: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(Some(b"stv-vault-v1"), &key[..]).expand(purpose, &mut out[..]).expect("32 bytes is a valid length");
    out
}

/// Tells the right key from a wrong one without opening the database, so "those aren't this vault's
/// words" and "the database is damaged" are different answers.
fn check_of(key: &Key) -> String {
    hex(&derive(key, b"check")[..16])
}

/// The vault's ID, as people see it ("7F3A-90C2"): the same from the key on any computer, so a recovery
/// kit, a backup copy and the vault in Settings can be matched up. It gives nothing about the key away.
fn fingerprint(key: &Key) -> String {
    let d = derive(key, b"fingerprint");
    format!("{:02X}{:02X}-{:02X}{:02X}", d[0], d[1], d[2], d[3])
}

/// The ID as it goes in a file name: "7F3A90C2".
fn file_id(key: &Key) -> String {
    fingerprint(key).replace('-', "")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DeviceSlot {
    /// How this device holds the key: "dpapi".
    method: String,
    wrapped: String,
    created: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct PasswordSlot {
    /// What the person wrote to remind themselves; shown where the password is asked for.
    hint: String,
    created: u64,
    /// Argon2id's memory in KiB, passes and lanes, kept so a later version can still open it.
    m: u32,
    t: u32,
    p: u32,
    salt: String,
    nonce: String,
    wrapped: String,
}

/// `keys.json`: the wrapped copies of the vault key, and what the person has done about recovery.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Keys {
    format: u32,
    /// The vault's own name, so a key file is never paired with another vault's database.
    vault: String,
    created: u64,
    /// The database's encryption, named so a later version opens it the way it was written.
    cipher: String,
    check: String,
    device: Option<DeviceSlot>,
    password: Option<PasswordSlot>,
    /// When the person said they had written the recovery words down; 0 if never.
    words_saved: u64,
    /// When they last proved they still have them.
    words_checked: u64,
    /// The reminder to save the words stays quiet until then.
    remind_after: u64,
}

fn read_keys(dir: &Path) -> Option<Keys> {
    // The copies are tried in turn: the file, the one kept beside it, and the one kept with the snapshots.
    [dir.join(KEYS), dir.join("keys.json.bak"), dir.join(SNAPSHOTS).join(KEYS)]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|t| serde_json::from_str::<Keys>(&t).ok())
        .find(|k| k.format >= 1 && !k.check.is_empty())
}

fn write_keys(dir: &Path, keys: &Keys) -> Result<(), String> {
    let text = serde_json::to_string_pretty(keys).map_err(e)?;
    let _ = std::fs::create_dir_all(dir.join(SNAPSHOTS));
    for (file, must) in [(dir.join(KEYS), true), (dir.join("keys.json.bak"), false), (dir.join(SNAPSHOTS).join(KEYS), false)] {
        let partial = file.with_extension("part");
        let done = std::fs::write(&partial, &text).and_then(|_| std::fs::rename(&partial, &file));
        if must {
            done.map_err(e)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
mod device {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    pub const METHOD: &str = "dpapi";

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
    }

    /// Takes what Windows allocated, wiping it first: one of the two is the vault key.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let bytes = slice.to_vec();
        slice.fill(0);
        LocalFree(out.pbData as _);
        bytes
    }

    /// Wraps the key so only this Windows account, on this computer, gets it back.
    pub fn wrap(key: &[u8], vault: &str) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe { CryptProtectData(&blob(key), std::ptr::null(), &blob(vault.as_bytes()), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 {
            return Err(format!("Windows wouldn't keep the vault's key: {}", std::io::Error::last_os_error()));
        }
        Ok(unsafe { take(out) })
    }

    pub fn unwrap(wrapped: &[u8], vault: &str) -> Option<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe { CryptUnprotectData(&blob(wrapped), std::ptr::null_mut(), &blob(vault.as_bytes()), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        (ok != 0).then(|| unsafe { take(out) })
    }
}

#[cfg(not(windows))]
mod device {
    pub const METHOD: &str = "";
    pub fn wrap(_key: &[u8], _vault: &str) -> Result<Vec<u8>, String> {
        Err("This device has no key store wired up yet.".into())
    }
    pub fn unwrap(_wrapped: &[u8], _vault: &str) -> Option<Vec<u8>> {
        None
    }
}

fn to_key(bytes: Vec<u8>) -> Option<Key> {
    let bytes = Zeroizing::new(bytes);
    let mut key = Zeroizing::new([0u8; 32]);
    (bytes.len() == 32).then(|| {
        key.copy_from_slice(&bytes);
        key
    })
}

fn device_slot(key: &Key, vault: &str) -> Result<DeviceSlot, String> {
    Ok(DeviceSlot { method: device::METHOD.into(), wrapped: b64(&device::wrap(&key[..], vault)?), created: now() })
}

fn from_device(keys: &Keys) -> Option<Key> {
    let slot = keys.device.as_ref()?;
    let key = to_key(device::unwrap(&unb64(&slot.wrapped).ok()?, &keys.vault)?)?;
    (check_of(&key) == keys.check).then_some(key)
}

/// The key a password stretches to. Slow on purpose: a password is short, so each guess must cost.
fn stretch(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = argon2::Params::new(m, t, p, Some(32)).map_err(e)?;
    let mut out = Zeroizing::new([0u8; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params).hash_password_into(password.as_bytes(), salt, &mut out[..]).map_err(e)?;
    Ok(out)
}

fn password_slot(key: &Key, vault: &str, password: &str, hint: &str) -> Result<PasswordSlot, String> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    let (m, t, p) = (64 * 1024, 3, 1);
    let salt: [u8; 16] = random()?;
    let nonce: [u8; 12] = random()?;
    let kek = stretch(password, &salt, m, t, p)?;
    let wrapped = aes_gcm::Aes256Gcm::new_from_slice(&kek[..])
        .map_err(e)?
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &key[..], aad: vault.as_bytes() })
        .map_err(|_| "The password couldn't be set.".to_string())?;
    Ok(PasswordSlot { hint: hint.trim().chars().take(120).collect(), created: now(), m, t, p, salt: b64(&salt), nonce: b64(&nonce), wrapped: b64(&wrapped) })
}

fn from_password(keys: &Keys, password: &str) -> Result<Key, String> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    const WRONG: &str = "That isn't the vault's password.";
    let slot = keys.password.as_ref().ok_or("This vault has no password.")?;
    let kek = stretch(password, &unb64(&slot.salt)?, slot.m, slot.t, slot.p)?;
    let nonce = unb64(&slot.nonce)?;
    if nonce.len() != 12 {
        return Err("The vault's key file is damaged.".into());
    }
    let plain = aes_gcm::Aes256Gcm::new_from_slice(&kek[..])
        .map_err(e)?
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &unb64(&slot.wrapped)?, aad: keys.vault.as_bytes() })
        .map_err(|_| WRONG.to_string())?;
    to_key(plain).filter(|k| check_of(k) == keys.check).ok_or_else(|| WRONG.to_string())
}

// ---------------------------------------------------------------------------------------------------------
// Recovery words

/// The vault key as 24 words (BIP39's English list, with its checksum, so a slip is caught).
fn words_of(key: &Key) -> Vec<String> {
    bip39::Mnemonic::from_entropy(&key[..]).expect("32 bytes make 24 words").words().map(str::to_string).collect()
}

/// Reads recovery words as people type them: any case, any spacing, numbered or not, and a word given by
/// its first four letters (which is enough to tell every word on the list apart).
fn key_from_words(text: &str) -> Result<Key, String> {
    let list = bip39::Language::English.word_list();
    let typed: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if typed.len() != 24 {
        return Err(format!("The recovery words are 24 words; that was {}.", typed.len()));
    }
    let mut unknown = Vec::new();
    let mut found = Vec::new();
    for (i, word) in typed.iter().enumerate() {
        if list.contains(&word.as_str()) {
            found.push(word.as_str());
            continue;
        }
        let stem: String = word.chars().take(4).collect();
        let like: Vec<&str> = list.iter().copied().filter(|w| stem.chars().count() == 4 && w.starts_with(&stem)).collect();
        match like.as_slice() {
            [only] => found.push(*only),
            _ => unknown.push(format!("{} (\"{word}\")", i + 1)),
        }
    }
    if !unknown.is_empty() {
        return Err(format!("Not on the word list: word {}. Check the spelling.", unknown.join(", word ")));
    }
    let phrase = Zeroizing::new(found.join(" "));
    let parsed = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, &phrase)
        .map_err(|_| "Those words don't fit together: one of them is wrong, or they're out of order.".to_string())?;
    to_key(parsed.to_entropy()).ok_or_else(|| "Those aren't a vault's recovery words.".to_string())
}

// ---------------------------------------------------------------------------------------------------------
// The database

const SCHEMA: &[&str] = &[
    // 1
    "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
     -- A note, whole, as `json`; the other columns are what the list and search read, so neither opens it.
     CREATE TABLE notes(
        id TEXT PRIMARY KEY, json TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
        archived INTEGER NOT NULL, status TEXT NOT NULL, title TEXT NOT NULL, preview TEXT NOT NULL,
        words INTEGER NOT NULL, search TEXT NOT NULL);
     CREATE INDEX notes_updated ON notes(updated);
     -- A note as it was: one row a version, added and never rewritten.
     CREATE TABLE versions(note TEXT NOT NULL, n INTEGER NOT NULL, json TEXT NOT NULL, PRIMARY KEY(note, n));
     -- What is written about a note: its chat, its analyses, and a translation for each language.
     CREATE TABLE docs(kind TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL, json TEXT NOT NULL,
        updated INTEGER NOT NULL, PRIMARY KEY(kind, owner, name));",
];

/// Opens a database file, with the key when it's sealed, and makes sure it really is readable.
fn connect(file: &Path, key: Option<&Key>) -> Result<Connection, String> {
    let conn = Connection::open(file).map_err(e)?;
    if let Some(key) = key {
        // A key given as raw bytes skips SQLCipher's own password stretching, which would otherwise add
        // a noticeable wait to every start.
        let pragma = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex(&derive(key, b"database")[..])));
        conn.execute_batch(&pragma).map_err(e)?;
        // Without SQLCipher the line above does nothing and the file would be written in the clear.
        let cipher: Option<String> = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).optional().map_err(e)?;
        if cipher.is_none() {
            return Err("This build can't encrypt the vault, so it won't open it.".into());
        }
        // Named, so a later SQLCipher with other defaults still reads what this one wrote.
        conn.execute_batch("PRAGMA cipher_compatibility = 4;").map_err(e)?;
    }
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(e)?;
    // The first read is where a wrong key or a damaged file shows.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(|_| "The database can't be read.".to_string())?;
    Ok(conn)
}

/// How the working database is set up: nothing written in the clear, and a save is on disk when it returns.
///
/// The journal is a rollback journal, not a write-ahead log. A log would hold the latest saves outside
/// the database file until it was next folded in, so a copy of `vault.db` alone (which is what people and
/// backup tools take) could be missing them. This way the one file is the whole vault after every save.
fn tune(conn: &Connection) -> Result<(), String> {
    conn.pragma_update(None, "temp_store", "MEMORY").map_err(e)?;
    let _: String = conn.query_row("PRAGMA journal_mode = TRUNCATE", [], |r| r.get(0)).map_err(e)?;
    conn.pragma_update(None, "synchronous", "FULL").map_err(e)?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let at: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(e)?;
    if at > SCHEMA.len() {
        return Err("This vault was written by a newer version of Voice. Update Voice to open it.".into());
    }
    for (i, step) in SCHEMA.iter().enumerate().skip(at) {
        let tx = conn.unchecked_transaction().map_err(e)?;
        tx.execute_batch(step).map_err(e)?;
        tx.pragma_update(None, "user_version", i + 1).map_err(e)?;
        tx.commit().map_err(e)?;
    }
    Ok(())
}

fn sound(conn: &Connection) -> bool {
    conn.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0)).is_ok_and(|r| r == "ok")
}

/// An empty database in memory with the vault's tables, for tests.
#[cfg(test)]
pub fn memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn
}

// ---------------------------------------------------------------------------------------------------------
// Snapshots

/// A sealed copy of the database, as offered to go back to.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub file: String,
    pub at: u64,
    pub bytes: u64,
}

fn snapshots(dir: &Path) -> Vec<Snapshot> {
    let mut all: Vec<Snapshot> = std::fs::read_dir(dir.join(SNAPSHOTS))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let at = file.strip_prefix("vault-")?.strip_suffix(".db")?.parse().ok()?;
            Some(Snapshot { file, at, bytes: entry.metadata().ok()?.len() })
        })
        .collect();
    all.sort_by(|a, b| b.at.cmp(&a.at));
    all
}

/// Which snapshots to keep, given their times newest first: the latest dozen, then the last of each day
/// for a fortnight, then the last of each month for a year.
fn worth_keeping(times: &[u64], now: u64) -> Vec<u64> {
    const DAY: u64 = 24 * 3600;
    let mut keep: Vec<u64> = times.iter().copied().take(12).collect();
    let mut seen = std::collections::HashSet::new();
    for &at in times {
        let age = now.saturating_sub(at);
        let slot = if age <= 14 * DAY { ("day", at / DAY) } else if age <= 366 * DAY { ("month", at / (30 * DAY)) } else { continue };
        if seen.insert(slot) && !keep.contains(&at) {
            keep.push(at);
        }
    }
    keep
}

fn modified(file: &Path) -> u64 {
    std::fs::metadata(file).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------------------------------------
// The vault itself

/// An open vault.
pub struct Vault {
    conn: Connection,
    key: Option<Key>,
    dir: PathBuf,
}

/// Why a vault isn't open, and what is still in hand to open it with.
pub struct Shut {
    /// The key, when this device released it but the database couldn't be read.
    key: Option<Key>,
    /// Set when the database is damaged or missing rather than locked.
    damaged: Option<String>,
}

impl Vault {
    /// Opens the vault in `dir`, making it the first time.
    pub fn open(dir: &Path) -> Result<Vault, Shut> {
        let fail = |why: String| Shut { key: None, damaged: Some(why) };
        std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
        let file = dir.join(DB);
        if !SEALED {
            return Vault::ready(dir, None).map_err(fail);
        }
        match (read_keys(dir), file.exists()) {
            (None, false) => {
                let key: Key = Zeroizing::new(random().map_err(fail)?);
                let vault = hex(&random::<8>().map_err(fail)?);
                let keys = Keys {
                    format: 1,
                    created: now(),
                    cipher: "sqlcipher4".into(),
                    check: check_of(&key),
                    device: Some(device_slot(&key, &vault).map_err(fail)?),
                    vault,
                    ..Default::default()
                };
                // The key is kept before anything is sealed with it.
                write_keys(dir, &keys).map_err(fail)?;
                Vault::ready(dir, Some(key)).map_err(fail)
            }
            // A database with no key file, as when only the database was copied to a new computer: the
            // recovery words open it.
            (None, true) => Err(Shut { key: None, damaged: None }),
            (Some(keys), exists) => {
                let key = from_device(&keys).ok_or(Shut { key: None, damaged: None })?;
                if !exists && !snapshots(dir).is_empty() {
                    return Err(Shut { key: Some(key), damaged: Some("The vault's database is missing.".into()) });
                }
                Vault::ready(dir, Some(key.clone())).map_err(|why| Shut { key: Some(key), damaged: Some(why) })
            }
        }
    }

    /// Opens the database with a key already in hand and brings it up to date.
    fn ready(dir: &Path, key: Option<Key>) -> Result<Vault, String> {
        let file = dir.join(DB);
        let conn = connect(&file, key.as_ref())?;
        tune(&conn)?;
        if std::fs::metadata(&file).map_or(0, |m| m.len()) <= CHECK_UP_TO && !sound(&conn) {
            return Err("The database is damaged.".into());
        }
        migrate(&conn)?;
        Ok(Vault { conn, key, dir: dir.into() })
    }

    /// Opens a vault this device can't, with a key from the recovery words or the password, and lets this
    /// device open it by itself from then on.
    fn open_with(dir: &Path, key: Key, by_words: bool) -> Result<Vault, Shut> {
        if let Some(keys) = read_keys(dir) {
            if keys.check != check_of(&key) {
                return Err(Shut { key: None, damaged: None });
            }
        }
        let vault = Vault::ready(dir, Some(key.clone())).map_err(|why| {
            // With no key file there's nothing to tell a wrong key from a damaged database.
            let known = read_keys(dir).is_some();
            Shut { key: known.then(|| key.clone()), damaged: known.then_some(why) }
        })?;
        let mut keys = read_keys(dir).unwrap_or_else(|| Keys { format: 1, created: now(), cipher: "sqlcipher4".into(), check: check_of(&key), ..Default::default() });
        if keys.vault.is_empty() {
            keys.vault = random::<8>().map(|b| hex(&b)).unwrap_or_else(|_| "vault".into());
            // A password slot is tied to the vault's name, so one from a lost file can't be carried over.
            keys.password = None;
        }
        if let Ok(slot) = device_slot(&key, &keys.vault) {
            keys.device = Some(slot);
        }
        // Words that just opened the vault are words the person has.
        if by_words {
            keys.words_checked = now();
            if keys.words_saved == 0 {
                keys.words_saved = now();
            }
        }
        let _ = write_keys(dir, &keys);
        Ok(vault)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// This vault's recovery words, for whoever holds it open (tests that make a second vault, as on
    /// another computer). None where vaults aren't sealed.
    pub fn recovery_words(&self) -> Option<Vec<String>> {
        self.key.as_ref().map(words_of)
    }


    fn change_keys(&self, change: impl FnOnce(&mut Keys) -> Result<(), String>) -> Result<(), String> {
        let mut keys = read_keys(&self.dir).ok_or("The vault's key file is missing.")?;
        change(&mut keys)?;
        write_keys(&self.dir, &keys)
    }

    fn changed_since(&self, at: u64) -> bool {
        let file = self.dir.join(DB);
        modified(&file) > at
    }

    /// Copies the database, sealed with the same key, and drops the copies no longer worth keeping.
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        self.snapshot_and_mirror(false)
    }

    /// `snapshot`, and with `mirror_now` a copy goes to the backup folder whether or not one went today.
    fn snapshot_and_mirror(&self, mirror_now: bool) -> Result<Snapshot, String> {
        let folder = self.dir.join(SNAPSHOTS);
        std::fs::create_dir_all(&folder).map_err(e)?;
        let at = now();
        let name = format!("vault-{at}.db");
        let partial = folder.join(format!("{name}.part"));
        let _ = std::fs::remove_file(&partial);
        {
            let mut copy = connect(&partial, self.key.as_ref())?;
            rusqlite::backup::Backup::new(&self.conn, &mut copy).map_err(e)?.run_to_completion(256, std::time::Duration::ZERO, None).map_err(e)?;
            if !sound(&copy) {
                let _ = std::fs::remove_file(&partial);
                return Err("The copy of the vault didn't read back.".into());
            }
        }
        let file = folder.join(&name);
        std::fs::rename(&partial, &file).map_err(e)?;
        let all = snapshots(&self.dir);
        let keep = worth_keeping(&all.iter().map(|s| s.at).collect::<Vec<_>>(), at);
        for old in all.iter().filter(|s| !keep.contains(&s.at)) {
            let _ = std::fs::remove_file(folder.join(&old.file));
        }
        if let Err(why) = self.mirror(&file, at, mirror_now) {
            eprintln!("The vault wasn't copied to its backup folder: {why}");
        }
        Ok(Snapshot { file: name, at, bytes: std::fs::metadata(&file).map_or(0, |m| m.len()) })
    }

    /// Once a day a sealed copy also goes to the backup folder (Documents › Voice-OS › Vault backups
    /// unless the person chose another), which is the folder people back up and sync: a copy that outlives
    /// this computer. Only when sealed, as it leaves the app's folder. Each copy is named with the vault's ID,
    /// and only this vault's own are thinned out, so copies of a vault set aside are never deleted by its
    /// replacement.
    fn mirror(&self, snapshot: &Path, at: u64, now: bool) -> Result<(), String> {
        const DAY: u64 = 24 * 3600;
        let Some(key) = self.key.as_ref().filter(|_| self.dir == default_dir()) else { return Ok(()) };
        let folder = backup_dir();
        let id = file_id(key);
        let mut kept: Vec<(u64, PathBuf)> = backups_in(&folder).into_iter().filter(|b| b.id == id).map(|b| (b.at, b.path)).collect();
        kept.sort();
        if !now && kept.last().is_some_and(|(newest, _)| at.saturating_sub(*newest) < DAY) {
            return Ok(());
        }
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let target = folder.join(format!("vault-{id}-{at}.db"));
        let partial = folder.join(format!("vault-{id}-{at}.db.part"));
        std::fs::copy(snapshot, &partial).and_then(|_| std::fs::rename(&partial, &target)).map_err(|e| format!("{}: {e}", folder.display()))?;
        let _ = std::fs::copy(self.dir.join(KEYS), folder.join(format!("keys-{id}.json")));
        let _ = std::fs::write(folder.join("About these files.txt"), ABOUT_BACKUPS);
        kept.push((at, target));
        for (_, old) in kept.iter().rev().skip(7) {
            let _ = std::fs::remove_file(old);
        }
        Ok(())
    }

    /// Copies the vault if it has changed and the last copy is old enough.
    fn snapshot_if_due(&self) {
        let newest = snapshots(&self.dir).first().map_or(0, |s| s.at);
        let has_notes = || self.conn.query_row("SELECT EXISTS(SELECT 1 FROM notes)", [], |r| r.get::<_, bool>(0)).unwrap_or(false);
        if now().saturating_sub(newest) >= SNAPSHOT_EVERY_S && self.changed_since(newest) && has_notes() {
            if let Err(why) = self.snapshot() {
                eprintln!("The vault wasn't copied: {why}");
            }
        }
    }
}

const ABOUT_BACKUPS: &str = "These are sealed copies of your vault from Voice-OS: your notes, books, bookmarks,\r\n\
chats, analyses and translations. The newest seven of each vault are kept.\r\n\r\n\
vault-7F3A90C2-1763200000.db is a copy of the vault whose ID is 7F3A-90C2, made at that time.\r\n\
keys-7F3A90C2.json goes with it: it isn't secret, and lets a vault password open the copy.\r\n\r\n\
A copy can't be read without that vault's 24 recovery words, its vault password if one was set, or the\r\n\
Windows account that made it. Your recovery kit says which vault ID its words are for.\r\n\r\n\
On a new computer: install Voice, then Settings > Vault > Bring in from a backup, and choose this folder.\r\n\
Meetings are kept in the folder above this one, sealed with the same key; the same words open them.\r\n";

/// Moves the database and its journal out of the way, never deleting them.
fn set_aside(dir: &Path, label: &str) -> Result<(), String> {
    let folder = dir.join("set-aside");
    let file = dir.join(DB);
    if !file.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&folder).map_err(e)?;
    let stamp = now();
    for ext in ["db", "db-journal", "db-wal", "db-shm"] {
        let from = file.with_extension(ext);
        if from.exists() {
            std::fs::rename(&from, folder.join(format!("vault-{stamp}-{label}.{ext}"))).map_err(|e| format!("The old database couldn't be moved aside: {e}"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// The app's one vault

enum State {
    Unopened,
    Open(Vault),
    Shut(Shut),
}

static STATE: Mutex<State> = Mutex::new(State::Unopened);

fn default_dir() -> PathBuf {
    crate::library::data_dir().join("vault")
}

/// Where the daily sealed copy goes: the folder the person chose, or Documents › Voice-OS › Vault
/// backups, beside the meetings.
pub fn backup_dir() -> PathBuf {
    let chosen = crate::prefs::load().vault_backup_dir;
    if chosen.trim().is_empty() { crate::library::root().join("Vault backups") } else { PathBuf::from(chosen.trim()) }
}

/// A sealed copy of a vault found in a backup folder.
#[derive(Debug, Clone, Serialize)]
pub struct Backup {
    /// The vault's ID as it goes in file names ("7F3A90C2").
    pub id: String,
    pub at: u64,
    #[serde(skip)]
    pub path: PathBuf,
    pub file: String,
    pub bytes: u64,
}

/// The vault copies in a folder, newest first: `vault-<ID>-<time>.db`.
fn backups_in(folder: &Path) -> Vec<Backup> {
    let mut all: Vec<Backup> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let (id, at) = file.strip_prefix("vault-")?.strip_suffix(".db")?.split_once('-')?;
            let (id, at): (String, u64) = (id.to_string(), at.parse().ok()?);
            (id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit())).then(|| Backup {
                id,
                at,
                bytes: entry.metadata().map_or(0, |m| m.len()),
                path: entry.path(),
                file,
            })
        })
        .collect();
    all.sort_by(|a, b| b.at.cmp(&a.at));
    all
}

fn state() -> std::sync::MutexGuard<'static, State> {
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    // A test that hasn't said where its files go would otherwise open, and bring notes into, the vault of
    // whoever is running it.
    let own = cfg!(test) && !crate::library::initialised();
    if matches!(*state, State::Unopened) && !own {
        let began = std::time::Instant::now();
        *state = match Vault::open(&default_dir()) {
            Ok(vault) => opened(vault),
            Err(shut) => closed(shut),
        };
        eprintln!("vault: opening took {} ms", began.elapsed().as_millis());
    }
    state
}

/// What was brought into the vault the first time it opened: the notes and books that were files before.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Moved {
    pub at: u64,
    pub notes: usize,
    pub books: usize,
    /// Notes and books that couldn't be brought in; their files are left where they were.
    pub failed: Vec<String>,
    /// The person has seen what was brought in.
    pub seen: bool,
}

fn meta_get<T: DeserializeOwned>(conn: &Connection, key: &str) -> Option<T> {
    let text: String = conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok()??;
    serde_json::from_str(&text).ok()
}

fn meta_put<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(e)?;
    conn.execute("INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", (key, json)).map_err(e)?;
    Ok(())
}

/// What happens each time the vault opens: notes and books still in files are brought in, and a copy is
/// made if due. The first time anything is brought in a copy is made at once, and goes to the backup folder,
/// so from the first minute there's a sealed copy that outlives this computer.
fn opened(vault: Vault) -> State {
    announce(&vault);
    let began = std::time::Instant::now();
    let notes = crate::notes::bring_in_files(&vault.conn);
    let books = crate::books::bring_in_files(&vault.conn);
    if notes.added + books.added > 0 {
        eprintln!("vault: brought in {} notes and {} books in {} ms", notes.added, books.added, began.elapsed().as_millis());
    }
    if notes.added + books.added > 0 || !notes.failed.is_empty() || !books.failed.is_empty() {
        let mut moved: Moved = meta_get(&vault.conn, "moved").unwrap_or_default();
        moved.at = now();
        moved.notes += notes.added;
        moved.books += books.added;
        moved.failed = notes.failed.iter().map(|id| format!("note {id}")).chain(books.failed.iter().map(|id| format!("book {id}"))).collect();
        moved.seen = false;
        let _ = meta_put(&vault.conn, "moved", &moved);
        if notes.added + books.added > 0 {
            if let Err(why) = vault.snapshot_and_mirror(true) {
                eprintln!("The vault wasn't copied after bringing notes in: {why}");
            }
        }
    }
    vault.snapshot_if_due();
    State::Open(vault)
}

/// A vault that didn't open: files sealed with its key can't be read or written until it does.
fn closed(shut: Shut) -> State {
    crate::sealed::set(None);
    State::Shut(shut)
}

fn unhex(text: &str) -> Option<Key> {
    let bytes: Option<Vec<u8>> = (0..text.len()).step_by(2).map(|i| text.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok())).collect();
    to_key(bytes?)
}

/// The keys of vaults brought into this one, kept in it so the files they sealed (meeting audio, the
/// pictures in their notes) open here too.
fn keyring(conn: &Connection) -> Vec<Key> {
    let kept: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = 'keyring'", [], |r| r.get(0)).optional().ok().flatten();
    let kept = Zeroizing::new(kept.unwrap_or_default());
    serde_json::from_str::<Vec<String>>(&kept).unwrap_or_default().iter().filter_map(|k| unhex(k)).collect()
}

fn add_to_keyring(conn: &Connection, key: &Key) -> Result<(), String> {
    let mut all: Vec<String> = keyring(conn).iter().map(|k| hex(&k[..])).collect();
    let new = hex(&key[..]);
    if !all.contains(&new) {
        all.push(new);
        let json = Zeroizing::new(serde_json::to_string(&all).map_err(e)?);
        conn.execute("INSERT INTO meta(key, value) VALUES('keyring', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1", [json.as_str()]).map_err(e)?;
    }
    Ok(())
}

/// Tells the files module which keys open files now: this vault's, then those brought into it.
fn announce(vault: &Vault) {
    crate::sealed::set(vault.key.as_ref().map(|own| {
        let mut ring = vec![crate::sealed::Opener::new(own)];
        ring.extend(keyring(&vault.conn).iter().filter(|k| k[..] != own[..]).map(|k| crate::sealed::Opener::new(k)));
        crate::sealed::Ring(ring)
    }));
}

/// Opens the vault if it hasn't been asked for yet.
pub fn touch() {
    drop(state());
}

/// Whether the vault is open (opening it first if nothing has asked yet).
pub fn is_open() -> bool {
    matches!(*state(), State::Open(_))
}

/// What to do each time the vault comes open, outside its lock: the example book, and sealing what was
/// written before the vault.
fn after_opening() {
    crate::books::install_example(None);
    crate::sealed::seal_existing();
}

/// Opens the vault as the app starts and keeps copies of it while the app runs. Files written before
/// the vault are sealed in the background.
///
/// None of it is done on the thread the app starts on: the window is up before the vault is open, and
/// whatever asks for a note first waits the few milliseconds that takes.
pub fn start() {
    std::thread::spawn(|| {
        let began = std::time::Instant::now();
        let open = is_open();
        eprintln!("vault: {} {} ms after the app started it", if open { "open" } else { "shut" }, began.elapsed().as_millis());
        if open {
            after_opening();
        }
    });
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(600));
        if let State::Open(vault) = &*state() {
            vault.snapshot_if_due();
        }
    });
}

/// Runs `f` with the database. Fails while the vault is shut.
pub fn with<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    match &*state() {
        State::Open(vault) => f(&vault.conn),
        _ => Err(LOCKED.into()),
    }
}

/// Like `with`, as one transaction: all of it is saved, or none.
pub fn write<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    with(|conn| {
        let tx = conn.unchecked_transaction().map_err(e)?;
        let out = f(&tx)?;
        tx.commit().map_err(e)?;
        Ok(out)
    })
}

pub fn get_doc<T: DeserializeOwned>(conn: &Connection, kind: &str, owner: &str, name: &str) -> Option<T> {
    let json: String = conn
        .prepare_cached("SELECT json FROM docs WHERE kind = ?1 AND owner = ?2 AND name = ?3")
        .and_then(|mut s| s.query_row((kind, owner, name), |r| r.get(0)).optional())
        .ok()??;
    serde_json::from_str(&json).ok()
}

pub fn put_doc<T: Serialize>(conn: &Connection, kind: &str, owner: &str, name: &str, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(e)?;
    conn.prepare_cached("INSERT INTO docs(kind, owner, name, json, updated) VALUES(?1, ?2, ?3, ?4, ?5) ON CONFLICT(kind, owner, name) DO UPDATE SET json = ?4, updated = ?5")
        .and_then(|mut s| s.execute((kind, owner, name, json, now() as i64)))
        .map_err(e)?;
    Ok(())
}

pub fn drop_doc(conn: &Connection, kind: &str, owner: &str, name: &str) -> Result<(), String> {
    conn.execute("DELETE FROM docs WHERE kind = ?1 AND owner = ?2 AND name = ?3", (kind, owner, name)).map_err(e)?;
    Ok(())
}

pub fn doc_names(conn: &Connection, kind: &str, owner: &str) -> Vec<String> {
    let read = || -> rusqlite::Result<Vec<String>> {
        conn.prepare_cached("SELECT name FROM docs WHERE kind = ?1 AND owner = ?2 ORDER BY name")?.query_map((kind, owner), |r| r.get(0))?.collect()
    };
    read().unwrap_or_default()
}

/// Everything that has a document of this kind: every book, for "book".
pub fn doc_names_of(conn: &Connection, kind: &str) -> Vec<String> {
    let read = || -> rusqlite::Result<Vec<String>> {
        conn.prepare_cached("SELECT DISTINCT owner FROM docs WHERE kind = ?1 ORDER BY owner")?.query_map([kind], |r| r.get(0))?.collect()
    };
    read().unwrap_or_default()
}

/// A note's document of one kind (its chat, its analyses, a translation), or the default when there's none.
pub fn doc<T: DeserializeOwned + Default>(kind: &str, owner: &str, name: &str) -> T {
    with(|conn| Ok(get_doc(conn, kind, owner, name))).ok().flatten().unwrap_or_default()
}

pub fn save_doc<T: Serialize>(kind: &str, owner: &str, name: &str, value: &T) -> Result<(), String> {
    with(|conn| put_doc(conn, kind, owner, name, value))
}

pub fn forget_doc(kind: &str, owner: &str, name: &str) -> Result<(), String> {
    with(|conn| drop_doc(conn, kind, owner, name))
}

// ---------------------------------------------------------------------------------------------------------
// What the page shows and asks for

#[derive(Debug, Default, Serialize)]
pub struct Status {
    /// "open", "locked" (this device can't release the key) or "damaged" (the database can't be read).
    pub state: &'static str,
    /// Whether the vault is encrypted on this platform.
    pub sealed: bool,
    pub detail: String,
    /// The vault's ID ("7F3A-90C2"), when it's open and sealed.
    pub id: String,
    pub notes: usize,
    pub books: usize,
    /// Notes of the person's own: not the chapters of a book kept for reading.
    pub own_notes: usize,
    pub words_saved: u64,
    pub words_checked: u64,
    /// Time to remind the person to save their recovery words.
    pub remind: bool,
    pub password: bool,
    pub hint: String,
    /// Whether the key is in hand, so a copy can be brought back without the words.
    pub key_known: bool,
    pub snapshots: Vec<Snapshot>,
    pub folder: String,
    /// Files written before the vault that are still being sealed.
    pub sealing: usize,
    /// What the first opening brought in, until the person has seen it.
    pub moved: Option<Moved>,
    /// Where the daily sealed copy goes, whether that was chosen, and the newest copy of this vault there.
    pub backup_dir: String,
    pub backup_chosen: bool,
    pub backup_latest: u64,
    /// Whether the backup folder can be written to now (a USB drive may be unplugged).
    pub backup_reachable: bool,
    /// Copies of other vaults in the backup folder, newest of each, for opening on a new computer.
    pub others: Vec<Backup>,
    /// Vaults whose keys this one holds, so what they sealed opens here.
    pub keyring: usize,
}

pub fn status() -> Status {
    let dir = default_dir();
    let keys = read_keys(&dir).unwrap_or_default();
    let backups = backup_dir();
    let chosen = !crate::prefs::load().vault_backup_dir.trim().is_empty();
    let mut status = Status {
        sealed: SEALED,
        words_saved: keys.words_saved,
        words_checked: keys.words_checked.max(keys.words_saved),
        password: keys.password.is_some(),
        hint: keys.password.as_ref().map(|p| p.hint.clone()).unwrap_or_default(),
        folder: dir.to_string_lossy().into_owned(),
        sealing: crate::sealed::left(),
        backup_dir: backups.to_string_lossy().into_owned(),
        backup_chosen: chosen,
        backup_reachable: backups.is_dir() || backups.parent().is_some_and(Path::is_dir),
        ..Default::default()
    };
    // The lock is taken before the folders are read, so a copy being made (as the vault first opens) is
    // finished and listed.
    let state = state();
    let found = backups_in(&backups);
    status.snapshots = snapshots(&dir);
    match &*state {
        State::Open(vault) => {
            status.state = "open";
            status.key_known = true;
            let count = |sql: &str| vault.conn.query_row(sql, [], |r| r.get::<_, usize>(0)).unwrap_or(0);
            status.notes = count("SELECT count(*) FROM notes");
            status.books = count("SELECT count(*) FROM docs WHERE kind = 'book'");
            let reading: std::collections::HashSet<String> = doc_names_of(&vault.conn, "book")
                .iter()
                .filter_map(|id| get_doc::<serde_json::Value>(&vault.conn, "book", id, ""))
                .filter(|b| b["editable"] == serde_json::Value::Bool(false))
                .flat_map(|b| b["notes"].as_array().cloned().unwrap_or_default())
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect();
            let ids: Vec<String> = vault.conn.prepare("SELECT id FROM notes").and_then(|mut s| s.query_map([], |r| r.get(0))?.collect()).unwrap_or_default();
            status.own_notes = ids.iter().filter(|id| !reading.contains(*id)).count();
            status.remind = SEALED && keys.words_saved == 0 && status.own_notes > 0 && now() >= keys.remind_after;
            status.moved = meta_get::<Moved>(&vault.conn, "moved").filter(|m| !m.seen);
            status.keyring = keyring(&vault.conn).len();
            if let Some(key) = &vault.key {
                status.id = fingerprint(key);
                let own = file_id(key);
                status.backup_latest = found.iter().find(|b| b.id == own).map_or(0, |b| b.at);
                let mut seen = vec![own];
                seen.extend(keyring(&vault.conn).iter().map(file_id));
                for b in &found {
                    if !seen.contains(&b.id) {
                        seen.push(b.id.clone());
                        status.others.push(b.clone());
                    }
                }
            }
        }
        State::Shut(shut) => {
            status.key_known = shut.key.is_some();
            status.state = if shut.damaged.is_some() { "damaged" } else { "locked" };
            status.detail = shut.damaged.clone().unwrap_or_default();
        }
        State::Unopened => {}
    }
    status
}

fn reopen(state: &mut State, opening: Result<Vault, Shut>, wrong: &str) -> Result<(), String> {
    match opening {
        Ok(vault) => {
            *state = opened(vault);
            Ok(())
        }
        Err(shut) => {
            let why = shut.damaged.clone().map(|d| format!("{d} Bring back an earlier copy below.")).unwrap_or_else(|| wrong.to_string());
            if shut.key.is_some() {
                *state = closed(shut);
            }
            Err(why)
        }
    }
}

/// Opens a shut vault with its recovery words.
pub fn unlock_with_words(text: &str) -> Result<(), String> {
    let key = key_from_words(text)?;
    {
        let mut state = state();
        if matches!(*state, State::Open(_)) {
            return Ok(());
        }
        reopen(&mut state, Vault::open_with(&default_dir(), key, true), "Those are recovery words, but for a different vault, not this one.")?;
    }
    std::thread::spawn(after_opening);
    Ok(())
}

/// Opens a shut vault with its password.
pub fn unlock_with_password(password: &str) -> Result<(), String> {
    {
        let mut state = state();
        if matches!(*state, State::Open(_)) {
            return Ok(());
        }
        let dir = default_dir();
        let key = from_password(&read_keys(&dir).ok_or("This vault has no password.")?, password)?;
        reopen(&mut state, Vault::open_with(&dir, key, false), "That isn't the vault's password.")?;
    }
    std::thread::spawn(after_opening);
    Ok(())
}

/// The vault ID that typed recovery words are for, so the person can tell which of several kits they hold.
pub fn id_of_words(text: &str) -> Result<String, String> {
    Ok(fingerprint(&key_from_words(text)?))
}

/// The recovery words, for the person to write down. Anyone who can ask already has the vault open.
pub fn words() -> Result<Vec<String>, String> {
    match &*state() {
        State::Open(Vault { key: Some(key), .. }) => Ok(words_of(key)),
        State::Open(_) => Err("Notes aren't encrypted on this device yet, so there are no recovery words.".into()),
        _ => Err(LOCKED.into()),
    }
}

fn open_vault<T>(f: impl FnOnce(&Vault) -> Result<T, String>) -> Result<T, String> {
    match &*state() {
        State::Open(vault) => f(vault),
        _ => Err(LOCKED.into()),
    }
}

/// Records that the person has the words written down.
pub fn words_saved() -> Result<(), String> {
    open_vault(|v| {
        v.change_keys(|k| {
            k.words_saved = now();
            k.words_checked = now();
            Ok(())
        })
    })
}

/// Checks words the person typed against the vault's, without changing anything but the date they
/// were last checked.
pub fn check_words(text: &str) -> Result<(), String> {
    let typed = key_from_words(text)?;
    open_vault(|v| {
        if v.key.as_ref().map(check_of) != Some(check_of(&typed)) {
            return Err(format!(
                "Those are recovery words, but for the vault {}, not this one ({}). Write this vault's words down again from Settings.",
                fingerprint(&typed),
                v.key.as_ref().map(fingerprint).unwrap_or_default()
            ));
        }
        v.change_keys(|k| {
            if k.words_saved == 0 {
                k.words_saved = now();
            }
            k.words_checked = now();
            Ok(())
        })
    })
}

/// Puts the reminder to save the words off for a month.
pub fn remind_later() -> Result<(), String> {
    open_vault(|v| {
        v.change_keys(|k| {
            k.remind_after = now() + 30 * 24 * 3600;
            Ok(())
        })
    })
}

/// The person has seen what the first opening brought in.
pub fn moved_seen() -> Result<(), String> {
    with(|conn| {
        if let Some(mut moved) = meta_get::<Moved>(conn, "moved") {
            moved.seen = true;
            meta_put(conn, "moved", &moved)?;
        }
        Ok(())
    })
}

/// Sets, changes or (with an empty password) removes the vault's password.
pub fn set_password(password: &str, hint: &str) -> Result<(), String> {
    open_vault(|v| {
        let key = v.key.as_ref().ok_or("Notes aren't encrypted on this device yet, so there's nothing for a password to open.")?;
        if password.is_empty() {
            return v.change_keys(|k| {
                k.password = None;
                Ok(())
            });
        }
        if password.chars().count() < 8 {
            return Err("Use at least 8 characters: the password is all that stands between a copy of the vault and your notes.".into());
        }
        if hint.to_lowercase().contains(&password.to_lowercase()) {
            return Err("The hint gives the password away. It's shown to anyone who has the vault.".into());
        }
        v.change_keys(|k| {
            k.password = Some(password_slot(key, &k.vault, password, hint)?);
            Ok(())
        })?;
        // The copy in the backup folder carries the key file the password needs, so it goes now.
        let _ = v.snapshot_and_mirror(true);
        Ok(())
    })
}

/// Chooses the folder the daily sealed copy goes to (empty for the default), and puts a copy there now, so
/// a folder that can't be written to is found at once.
pub fn set_backup_dir(folder: &str) -> Result<(), String> {
    let folder = folder.trim();
    if !folder.is_empty() {
        let path = Path::new(folder);
        if !path.is_dir() {
            return Err("That folder can't be found.".into());
        }
        if path.starts_with(default_dir()) {
            return Err("Choose a folder outside the vault's own, ideally one that is backed up or synced, or a USB drive.".into());
        }
        let probe = path.join(".voice-backup-check");
        std::fs::write(&probe, b"ok").map_err(|e| format!("Voice can't write to that folder: {e}"))?;
        let _ = std::fs::remove_file(probe);
    }
    crate::prefs::update(|p| p.vault_backup_dir = folder.to_string())?;
    open_vault(|v| v.snapshot_and_mirror(true).map(drop))
}

/// The recovery kit: the words and what to do with them, as text to save or print.
pub fn kit() -> Result<String, String> {
    let words = words()?;
    let status = status();
    let date = |at: u64| {
        let days = at / 86_400;
        // Civil date from days since 1970 (Howard Hinnant's algorithm), so the kit needs no clock library.
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y}-{m:02}-{d:02}")
    };
    let mut out = String::from("Voice-OS: recovery kit\r\n");
    out += &format!("Vault ID {}, made {}\r\n\r\n", status.id, date(now()));
    out += "These 24 words open your vault: your notes, books, bookmarks and meetings. Anyone who has them, and a\r\n";
    out += "copy of the vault, can read everything in it, so keep this somewhere private: printed and filed, or in a\r\n";
    out += "password manager. Not only on this computer, and not in the same folder as the vault.\r\n\r\n";
    for (i, row) in words.chunks(4).enumerate() {
        let line: Vec<String> = row.iter().enumerate().map(|(j, w)| format!("{:>2}. {:<10}", i * 4 + j + 1, w)).collect();
        out += &format!("  {}\r\n", line.join(" ").trim_end());
    }
    out += "\r\nThe first four letters of each word are enough. Capitals and spacing don't matter.\r\n\r\n";
    out += "IF VOICE ASKS FOR THEM ON THIS COMPUTER\r\n";
    out += "Voice opens the vault by itself day to day. After Windows is reinstalled, or your Windows password is\r\n";
    out += "reset by someone else, it can't, and asks for these words (or your vault password, if you set one).\r\n\r\n";
    out += "ON A NEW COMPUTER\r\n";
    out += "1. Copy the Voice-OS folder from Documents (or wherever your backups are) to the new computer, or\r\n";
    out += "   let OneDrive or your backup bring it back.\r\n";
    out += "2. Install Voice, open Settings > Vault, and choose Bring in from a backup.\r\n";
    out += "3. Choose that folder, type these words, and choose Use it on this computer. Your notes, books and\r\n";
    out += "   meetings come back, and these words keep working.\r\n\r\n";
    out += "IF YOU FORGET YOUR VAULT PASSWORD\r\n";
    out += "Open the vault with these words, then set a new password in Settings > Vault.\r\n\r\n";
    out += "WHERE THINGS ARE\r\n";
    out += &format!("- The vault: {}\r\n", status.folder);
    out += &format!("- Sealed copies, made daily: {}\r\n", status.backup_dir);
    out += &format!("  (files named vault-{}-....db belong to this vault)\r\n", status.id.replace('-', ""));
    out += &format!("- Meetings, sealed with the same key: {}\r\n", crate::library::root().display());
    if status.password {
        out += "- A vault password is set";
        out += &if status.hint.is_empty() { ".\r\n".to_string() } else { format!(", with the hint: {}\r\n", status.hint) };
    }
    Ok(out)
}

/// Makes a sealed copy now, and puts it in the backup folder too, as before something the person isn't
/// sure of.
pub fn copy_now() -> Result<(), String> {
    open_vault(|v| v.snapshot_and_mirror(true).map(drop))
}

/// Brings back a snapshot in place of the database, which is set aside rather than deleted.
pub fn restore(file: &str) -> Result<(), String> {
    let dir = default_dir();
    let chosen = snapshots(&dir).into_iter().find(|s| s.file == file).ok_or("That copy is no longer kept.")?;
    let source = dir.join(SNAPSHOTS).join(&chosen.file);
    let mut state = state();
    let key = match &*state {
        State::Open(vault) => {
            // Going back is itself undoable: how the vault is now is kept as a copy first.
            vault.snapshot()?;
            vault.key.clone()
        }
        State::Shut(shut) if shut.key.is_some() || !SEALED => shut.key.clone(),
        _ => return Err("Open the vault with your recovery words first; then a copy can be brought back.".into()),
    };
    if !connect(&source, key.as_ref()).is_ok_and(|c| sound(&c)) {
        return Err("That copy can't be read either. Try an earlier one.".into());
    }
    // The connection is closed before its file is moved.
    *state = closed(Shut { key: key.clone(), damaged: Some("A copy is being brought back.".into()) });
    set_aside(&dir, "replaced")?;
    std::fs::copy(&source, dir.join(DB)).map_err(e)?;
    let vault = Vault::ready(&dir, key.clone()).map_err(|why| {
        *state = closed(Shut { key: key.clone(), damaged: Some(why.clone()) });
        why
    })?;
    *state = opened(vault);
    Ok(())
}

/// Sets the vault that can't be opened aside, untouched, and starts an empty one. The old one can still be
/// brought in later if its words turn up.
pub fn start_new() -> Result<(), String> {
    let dir = default_dir();
    {
        let mut state = state();
        if matches!(*state, State::Open(_)) {
            return Err("The vault is open; there's nothing to replace.".into());
        }
        *state = closed(Shut { key: None, damaged: None });
        set_aside(&dir, "unopened")?;
        let folder = dir.join("set-aside");
        let stamp = now();
        for name in [KEYS, "keys.json.bak"] {
            let _ = std::fs::rename(dir.join(name), folder.join(format!("{name}-{stamp}")));
        }
        // Copies of the old vault are sealed with the old key, so they go with it.
        let _ = std::fs::rename(dir.join(SNAPSHOTS), folder.join(format!("snapshots-{stamp}")));
        reopen(&mut state, Vault::open(&dir), "A new vault couldn't be started.")?;
    }
    std::thread::spawn(after_opening);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Bringing in another vault, a backup folder, or an old copy of Voice's folders

/// What bringing things in did, to tell the person.
#[derive(Debug, Default, Clone, Serialize)]
pub struct BroughtIn {
    pub notes: usize,
    pub books: usize,
    pub meetings: usize,
    /// Found, but already here, so left as they are here.
    pub had: usize,
    /// Couldn't be brought in: what, and why.
    pub failed: Vec<String>,
    /// The vault file that was read, if one was.
    pub vault_file: String,
    /// The ID of the vault whose key now opens what it sealed here, when it isn't this one.
    pub other_vault: String,
    /// The vault was taken on as this computer's own (`adopt`): its words are the words from now on.
    pub adopted: bool,
}

/// The key that opens a vault file: from its recovery words, or from its password and the key file kept
/// beside it in a backup folder (`keys-<ID>.json`, or `keys.json`).
fn key_for(file: &Path, words: &str, password: &str) -> Result<Option<(Key, Option<Keys>)>, String> {
    let beside = || -> Option<Keys> {
        let folder = file.parent()?;
        let name = file.file_name()?.to_str()?;
        let id = name.strip_prefix("vault-").and_then(|n| n.split_once('-')).map(|(id, _)| id.to_string());
        let mut candidates = Vec::new();
        if let Some(id) = id {
            candidates.push(folder.join(format!("keys-{id}.json")));
        }
        candidates.push(folder.join(KEYS));
        candidates.push(folder.parent()?.join(KEYS));
        candidates.iter().filter_map(|p| std::fs::read_to_string(p).ok()).filter_map(|t| serde_json::from_str::<Keys>(&t).ok()).find(|k| !k.check.is_empty())
    };
    if !words.trim().is_empty() {
        let key = key_from_words(words)?;
        let keys = beside().filter(|k| k.check == check_of(&key));
        return Ok(Some((key, keys)));
    }
    if !password.is_empty() {
        let keys = beside().ok_or("There's no key file beside that copy, so its password can't open it. Use its recovery words.")?;
        let key = from_password(&keys, password)?;
        return Ok(Some((key, Some(keys))));
    }
    Ok(None)
}

/// Opens a copy of a vault file to read, so the file chosen is never written to (opening can write a journal).
fn open_copy(dir: &Path, file: &Path, key: Option<&Key>) -> Result<(Connection, PathBuf), String> {
    let copy = dir.join(format!("bring-in-{}.tmp", now()));
    std::fs::copy(file, &copy).map_err(|e| format!("That file couldn't be read: {e}"))?;
    match connect(&copy, key) {
        Ok(conn) => Ok((conn, copy)),
        Err(_) => {
            remove_copy(&copy);
            Err(if SEALED { "That file didn't open. It needs the recovery words (or password) of the vault it came from.".to_string() } else { "That file isn't a notes vault.".to_string() })
        }
    }
}

fn remove_copy(copy: &Path) {
    for ext in ["tmp", "tmp-wal", "tmp-shm", "tmp-journal"] {
        let _ = std::fs::remove_file(copy.with_extension(ext));
    }
}

/// Merges another vault's notes and books into this one's database. Notes and books already here are left
/// as they are; versions, chats, bookmarks and the rest come only with notes that weren't here, so nothing
/// here is mixed with another vault's history of the same note.
fn merge(into: &Connection, other: &Connection, out: &mut BroughtIn) -> Result<(), String> {
    let tx = into.unchecked_transaction().map_err(e)?;
    let count = |sql: &str| tx.query_row(sql, [], |r| r.get::<_, usize>(0)).map_err(e);
    let (notes, books) = (count("SELECT count(*) FROM notes")?, count("SELECT count(*) FROM docs WHERE kind = 'book'")?);
    let found: usize = other.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).map_err(|_| "That file isn't a notes vault.".to_string())?;
    let found_books: usize = other.query_row("SELECT count(*) FROM docs WHERE kind = 'book'", [], |r| r.get(0)).unwrap_or(0);
    // Copies a table of the other vault's into one of this vault's, leaving rows already here alone.
    let copy_rows = |from: &str, to: &str, columns: &str| -> Result<(), String> {
        let n = columns.split(',').count();
        let marks = vec!["?"; n].join(",");
        let mut read = other.prepare(&format!("SELECT {columns} FROM {from}")).map_err(|_| "That file isn't a notes vault.".to_string())?;
        let mut put = tx.prepare(&format!("INSERT OR IGNORE INTO {to}({columns}) VALUES({marks})")).map_err(e)?;
        let mut rows = read.query([]).map_err(e)?;
        while let Some(row) = rows.next().map_err(e)? {
            let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i)).collect::<Result<_, _>>().map_err(e)?;
            put.execute(rusqlite::params_from_iter(values)).map_err(e)?;
        }
        Ok(())
    };
    tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS had(id TEXT PRIMARY KEY); DELETE FROM had; INSERT INTO had SELECT id FROM notes;").map_err(e)?;
    copy_rows("notes", "notes", "id,json,created,updated,archived,status,title,preview,words,search")?;
    tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS v_in(note TEXT, n INTEGER, json TEXT); DELETE FROM v_in; CREATE TEMP TABLE IF NOT EXISTS d_in(kind TEXT, owner TEXT, name TEXT, json TEXT, updated INTEGER); DELETE FROM d_in;").map_err(e)?;
    copy_rows("versions", "v_in", "note,n,json")?;
    copy_rows("docs", "d_in", "kind,owner,name,json,updated")?;
    tx.execute_batch(
        "INSERT OR IGNORE INTO versions(note, n, json) SELECT note, n, json FROM v_in WHERE note NOT IN (SELECT id FROM had);
         INSERT OR IGNORE INTO docs(kind, owner, name, json, updated) SELECT kind, owner, name, json, updated FROM d_in WHERE owner NOT IN (SELECT id FROM had);
         DROP TABLE had; DROP TABLE v_in; DROP TABLE d_in;",
    )
    .map_err(e)?;
    let (notes_after, books_after) = (count("SELECT count(*) FROM notes")?, count("SELECT count(*) FROM docs WHERE kind = 'book'")?);
    tx.commit().map_err(e)?;
    out.notes += notes_after - notes;
    out.books += books_after - books;
    out.had += (found + found_books).saturating_sub(notes_after - notes + books_after - books);
    Ok(())
}

/// Keeps the keys another vault held for vaults brought into it, so what those sealed opens here too.
fn carry_keyring(into: &Connection, from: &Connection) -> Result<(), String> {
    for key in keyring(from) {
        add_to_keyring(into, &key)?;
    }
    Ok(())
}

/// Keeps another vault's key in this one, so its meetings and the pictures in its notes open here.
fn keep_key(vault: &Vault, other: &Key, out: &mut BroughtIn) -> Result<(), String> {
    if vault.key.as_ref().is_some_and(|own| own[..] == other[..]) {
        return Ok(());
    }
    add_to_keyring(&vault.conn, other)?;
    announce(vault);
    out.other_vault = fingerprint(other);
    Ok(())
}

/// Brings the notes and books of another vault file into this one: a sealed copy from a backup, a vault
/// set aside, or one from another computer. Notes already here are left as they are. `words` (or
/// `password`, with the key file beside a backup) open that vault, and can be left out when it shares this
/// vault's key.
pub fn bring_in(file: &Path, words: &str, password: &str) -> Result<BroughtIn, String> {
    let found = key_for(file, words, password)?;
    let mut out = BroughtIn { vault_file: file.to_string_lossy().into_owned(), ..Default::default() };
    {
        let state = state();
        let State::Open(vault) = &*state else { return Err(LOCKED.into()) };
        let key = if SEALED { Some(found.as_ref().map(|(k, _)| k.clone()).or_else(|| vault.key.clone()).ok_or(LOCKED)?) } else { None };
        let (other, copy) = open_copy(&vault.dir, file, key.as_ref())?;
        let merged = merge(&vault.conn, &other, &mut out).and_then(|_| carry_keyring(&vault.conn, &other));
        drop(other);
        remove_copy(&copy);
        merged?;
        if let Some(other) = &key {
            keep_key(vault, other, &mut out)?;
        }
    }
    std::thread::spawn(crate::sealed::seal_existing);
    Ok(out)
}

/// Keeps another vault's key from its recovery words alone, so the meetings and note files it sealed open
/// here: for meetings copied from another computer, or a vault whose database is lost.
pub fn add_words(words: &str) -> Result<String, String> {
    let key = key_from_words(words)?;
    let id = fingerprint(&key);
    open_vault(|vault| {
        if vault.key.as_ref().is_some_and(|own| own[..] == key[..]) {
            return Err("Those are this vault's own words; what it sealed already opens.".into());
        }
        let mut out = BroughtIn::default();
        keep_key(vault, &key, &mut out)
    })?;
    std::thread::spawn(crate::sealed::seal_existing);
    Ok(id)
}

/// The newest vault file in a folder chosen to bring in from: a backup folder (`vault-<ID>-<time>.db`), a
/// vault's own folder (`vault.db`, with its `snapshots`), or Voice's data folder (`vault/vault.db`).
fn vault_file_in(folder: &Path, words: &str) -> Option<PathBuf> {
    let wanted = (!words.trim().is_empty()).then(|| key_from_words(words).ok()).flatten().map(|k| file_id(&k));
    let mut candidates: Vec<Backup> = backups_in(folder);
    candidates.extend(backups_in(&folder.join("Vault backups")));
    if let Some(id) = &wanted {
        // With words given, a copy of that vault is chosen over the newest of another.
        if candidates.iter().any(|b| &b.id == id) {
            candidates.retain(|b| &b.id == id);
        }
    }
    candidates.sort_by(|a, b| b.at.cmp(&a.at));
    let plain = [folder.join(DB), folder.join("vault").join(DB)].into_iter().find(|p| p.is_file());
    match (plain, candidates.first()) {
        // The live database is newer than any daily copy of it.
        (Some(db), _) => Some(db),
        (None, Some(b)) => Some(b.path.clone()),
        _ => None,
    }
}

/// Copies meeting folders (`session-*`) from `from` into the library, leaving any already here alone.
/// Sealed files are copied as they are; plain ones are sealed in the background afterwards.
fn copy_meetings(from: &Path, out: &mut BroughtIn) {
    let root = crate::library::root();
    if from == root {
        return;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(from).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).filter(|n| n.starts_with("session-")).map(str::to_string) else { continue };
        let target = root.join(&name);
        if target.exists() {
            out.had += 1;
            continue;
        }
        // Copied under another name first, so neither the list of meetings nor sealing sees it half there.
        let partial = root.join(format!("importing-{name}"));
        let _ = std::fs::remove_dir_all(&partial);
        match copy_tree(&dir, &partial).and_then(|_| std::fs::rename(&partial, &target)) {
            Ok(()) => out.meetings += 1,
            Err(why) => {
                let _ = std::fs::remove_dir_all(&partial);
                out.failed.push(format!("meeting {name}: {why}"));
            }
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(&path, to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Brings in everything Voice keeps that's in a folder: a backup folder, a copy of Documents › Superteem
/// Voice from another computer, or a copy of Voice's own data folder from before the vault. Finds the newest
/// vault copy (opened with `words` or `password`), notes and books kept as files before the vault, and
/// meeting folders. Nothing in the folder is changed.
pub fn bring_in_folder(folder: &Path, words: &str, password: &str) -> Result<BroughtIn, String> {
    if !folder.is_dir() {
        return Err("That folder can't be found.".into());
    }
    let file = vault_file_in(folder, words);
    let mut out = match &file {
        Some(file) => bring_in(file, words, password)?,
        None => BroughtIn::default(),
    };
    // Words given without a vault file still open what that vault sealed: meetings copied here.
    if file.is_none() && !words.trim().is_empty() {
        if let Ok(id) = add_words(words) {
            out.other_vault = id;
        }
    }
    bring_in_extras(folder, &mut out)?;
    if out.notes + out.books + out.meetings + out.had == 0 && out.failed.is_empty() && out.other_vault.is_empty() {
        return Err("Nothing from Voice was found in that folder. Choose the Voice-OS (or earlier Superteem Voice) folder from Documents, a Vault backups folder, or Voice's own data folder.".into());
    }
    std::thread::spawn(crate::sealed::seal_existing);
    Ok(out)
}

/// What's in a folder besides a vault: notes and books kept as files before the vault, and meetings.
fn bring_in_extras(folder: &Path, out: &mut BroughtIn) -> Result<(), String> {
    // Notes and books kept as files before the vault: Voice's data folder holds them in `notes` and `books`,
    // or one of those folders was chosen itself.
    let named = |name: &str| if folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) { folder.to_path_buf() } else { folder.join(name) };
    with(|conn| {
        let got = crate::notes::bring_in_folder(conn, &named("notes"), false);
        out.notes += got.added;
        out.had += got.had;
        out.failed.extend(got.failed.into_iter().map(|id| format!("note {id}")));
        let got = crate::books::bring_in_folder(conn, &named("books"), false);
        out.books += got.added;
        out.had += got.had;
        out.failed.extend(got.failed.into_iter().map(|id| format!("book {id}")));
        Ok(())
    })?;
    copy_meetings(folder, out);
    copy_meetings(&folder.join("Voice-OS"), out);
    // Versions up to 0.10 were Voice by Superteem, whose folder this was.
    copy_meetings(&folder.join("Superteem Voice"), out);
    // A backup folder sits inside the meetings' folder: they're beside it.
    if folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Vault backups")) {
        if let Some(parent) = folder.parent() {
            copy_meetings(parent, out);
        }
    }
    Ok(())
}

/// Takes on another vault as this computer's own, as on a new computer: its database takes the place of
/// this one, this computer is given its own copy of its key, and its recovery words (and password) go on
/// working. Nothing here is lost: this vault is set aside, its notes and books are merged into the one taken
/// on, and its key is kept so what it sealed still opens.
pub fn adopt(chosen: &Path, words: &str, password: &str) -> Result<BroughtIn, String> {
    // A folder: the newest copy of the vault in it, and then its meetings and older files.
    let folder = chosen.is_dir().then(|| chosen.to_path_buf());
    let file = match &folder {
        Some(folder) => vault_file_in(folder, words).ok_or("There's no vault in that folder. Choose a Vault backups folder, or a vault file.")?,
        None => chosen.to_path_buf(),
    };
    let mut out = adopt_file(&file, words, password)?;
    if let Some(folder) = folder {
        // The vault taken on is counted whole; notes, books and meetings found beside it are added.
        bring_in_extras(&folder, &mut out)?;
        std::thread::spawn(crate::sealed::seal_existing);
    }
    Ok(out)
}

fn adopt_file(file: &Path, words: &str, password: &str) -> Result<BroughtIn, String> {
    let (key, beside) = key_for(file, words, password)?.ok_or("Type that vault's recovery words, or its password.")?;
    if !SEALED {
        return Err("Vaults aren't encrypted on this device yet; bring its notes in instead.".into());
    }
    let dir = default_dir();
    let mut out = BroughtIn { vault_file: file.to_string_lossy().into_owned(), adopted: true, ..Default::default() };
    {
        let mut state = state();
        let State::Open(current) = &*state else { return Err(LOCKED.into()) };
        if current.key.as_ref().is_some_and(|own| own[..] == key[..]) {
            return Err("That's a copy of this vault. To go back to it, use Bring back a copy.".into());
        }
        // The copy must open before anything here is moved.
        let (probe, copy) = open_copy(&dir, file, Some(&key))?;
        let readable = sound(&probe);
        drop(probe);
        if !readable {
            remove_copy(&copy);
            return Err("That copy is damaged. Try an earlier one.".into());
        }
        current.snapshot()?;
        let was = current.key.clone();
        *state = closed(Shut { key: None, damaged: Some("Another vault is being taken on.".into()) });
        // This vault, set aside whole, so it can be merged back below and brought back if anything fails.
        let stamp = now();
        let aside = dir.join("set-aside");
        std::fs::create_dir_all(&aside).map_err(e)?;
        let old_db = aside.join(format!("vault-{stamp}-before-taking-on.db"));
        std::fs::rename(dir.join(DB), &old_db).map_err(|e| format!("This vault couldn't be moved aside: {e}"))?;
        for name in [KEYS, "keys.json.bak"] {
            let _ = std::fs::rename(dir.join(name), aside.join(format!("{name}-{stamp}")));
        }
        let _ = std::fs::rename(dir.join(SNAPSHOTS), aside.join(format!("snapshots-{stamp}")));
        let put_back = |why: String| -> String {
            let _ = std::fs::remove_file(dir.join(DB));
            let _ = std::fs::rename(&old_db, dir.join(DB));
            let _ = std::fs::rename(aside.join(format!("{KEYS}-{stamp}")), dir.join(KEYS));
            let _ = std::fs::rename(aside.join(format!("keys.json.bak-{stamp}")), dir.join("keys.json.bak"));
            let _ = std::fs::rename(aside.join(format!("snapshots-{stamp}")), dir.join(SNAPSHOTS));
            why
        };
        if let Err(why) = std::fs::rename(&copy, dir.join(DB)) {
            remove_copy(&copy);
            let why = put_back(format!("That vault couldn't be put in place: {why}"));
            *state = match Vault::open(&dir) {
                Ok(vault) => opened(vault),
                Err(shut) => closed(shut),
            };
            return Err(why);
        }
        // Its key file comes with it when it was beside the copy, so its password still opens it.
        let mut keys = beside.unwrap_or_else(|| Keys { format: 1, created: now(), cipher: "sqlcipher4".into(), check: check_of(&key), ..Default::default() });
        if keys.vault.is_empty() {
            keys.vault = hex(&random::<8>()?);
            keys.password = None;
        }
        keys.device = Some(device_slot(&key, &keys.vault)?);
        // Words that just opened it are words the person has.
        if !words.trim().is_empty() {
            if keys.words_saved == 0 {
                keys.words_saved = now();
            }
            keys.words_checked = now();
        }
        keys.remind_after = 0;
        write_keys(&dir, &keys)?;
        let vault = match Vault::ready(&dir, Some(key.clone())) {
            Ok(vault) => vault,
            Err(why) => {
                let why = put_back(format!("That vault didn't open here: {why}"));
                *state = match Vault::open(&dir) {
                    Ok(vault) => opened(vault),
                    Err(shut) => closed(shut),
                };
                return Err(why);
            }
        };
        // What this computer had, merged in, and its key kept for the meetings it sealed.
        if let Some(was) = &was {
            if let Ok(old) = connect(&old_db, Some(was)) {
                if let Err(why) = merge(&vault.conn, &old, &mut out).and_then(|_| carry_keyring(&vault.conn, &old)) {
                    out.failed.push(format!("this computer's notes: {why}"));
                }
            }
            let _ = add_to_keyring(&vault.conn, was);
        }
        out.notes = vault.conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).unwrap_or(0);
        out.books = vault.conn.query_row("SELECT count(*) FROM docs WHERE kind = 'book'", [], |r| r.get(0)).unwrap_or(0);
        out.other_vault = fingerprint(&key);
        *state = opened(vault);
    }
    std::thread::spawn(after_opening);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stv-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn add_note(conn: &Connection, id: &str, text: &str) {
        conn.execute("INSERT INTO notes(id, json, created, updated, archived, status, title, preview, words, search) VALUES(?1, ?2, 1, 1, 0, '', ?2, '', 1, ?2)", (id, text)).unwrap();
    }

    fn titles(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT title FROM notes ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn recovery_words_are_the_key_however_they_are_typed() {
        let key: Key = Zeroizing::new([7u8; 32]);
        let words = words_of(&key);
        assert_eq!(words.len(), 24);
        assert_eq!(key_from_words(&words.join(" ")).unwrap()[..], key[..]);
        // Numbered, on lines, in capitals, and cut to four letters.
        let messy: String = words.iter().enumerate().map(|(i, w)| format!("{}. {}\n", i + 1, w.chars().take(4).collect::<String>().to_uppercase())).collect();
        assert_eq!(key_from_words(&messy).unwrap()[..], key[..]);
        assert!(key_from_words(&words[..23].join(" ")).unwrap_err().contains("that was 23"));
        let mut slip = words.clone();
        slip[4] = "zzzz".into();
        assert!(key_from_words(&slip.join(" ")).unwrap_err().contains("word 5 (\"zzzz\")"));
        let mut swapped = words.clone();
        swapped.swap(0, 1);
        if swapped != words {
            assert!(key_from_words(&swapped.join(" ")).unwrap_err().contains("out of order"));
        }
    }

    #[test]
    fn a_password_wraps_the_key_and_a_wrong_one_is_told() {
        let key: Key = Zeroizing::new([9u8; 32]);
        let mut keys = Keys { format: 1, vault: "abc".into(), check: check_of(&key), ..Default::default() };
        // Small Argon2 settings keep the test quick; the slot records its own.
        let mut slot = password_slot(&key, "abc", "correct horse", "the usual animal").unwrap();
        assert_eq!((slot.m, slot.t, slot.hint.as_str()), (64 * 1024, 3, "the usual animal"));
        slot.m = 64;
        slot.t = 1;
        let kek = stretch("correct horse", &unb64(&slot.salt).unwrap(), slot.m, slot.t, slot.p).unwrap();
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        let nonce = unb64(&slot.nonce).unwrap();
        slot.wrapped = b64(&aes_gcm::Aes256Gcm::new_from_slice(&kek[..]).unwrap().encrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &key[..], aad: b"abc" }).unwrap());
        keys.password = Some(slot);
        assert_eq!(from_password(&keys, "correct horse").unwrap()[..], key[..]);
        assert_eq!(from_password(&keys, "wrong horse").unwrap_err(), "That isn't the vault's password.");
        // A slot moved to another vault's key file doesn't open.
        keys.vault = "other".into();
        assert!(from_password(&keys, "correct horse").is_err());
    }

    #[test]
    fn the_oldest_snapshots_thin_out() {
        const DAY: u64 = 24 * 3600;
        let now = 1000 * DAY;
        // One an hour for three days, then one a day back to 400 days ago.
        let mut times: Vec<u64> = (0..72).map(|h| now - h * 3600).collect();
        times.extend((3..400).map(|d| now - d * DAY));
        let keep = worth_keeping(&times, now);
        assert!(times[..12].iter().all(|t| keep.contains(t)), "the latest dozen are all kept");
        assert!(keep.contains(&(now - 10 * DAY)), "one a day for a fortnight");
        assert!(!keep.contains(&(now - 399 * DAY)), "nothing over a year old");
        assert!(keep.len() < 45, "kept {}", keep.len());
    }

    #[test]
    fn a_vault_opens_again_and_its_file_is_sealed() {
        let dir = folder("open");
        {
            let vault = Vault::open(&dir).ok().expect("a new vault opens");
            add_note(vault.conn(), "n1", "The launch is on Friday");
        }
        let vault = Vault::open(&dir).ok().expect("it opens again");
        assert_eq!(titles(vault.conn()), ["The launch is on Friday"]);
        drop(vault);
        let raw = std::fs::read(dir.join(DB)).unwrap();
        let plain = |bytes: &[u8]| bytes.windows(6).any(|w| w == b"launch") || bytes.starts_with(b"SQLite format 3");
        assert_eq!(plain(&raw), !SEALED, "the file is readable exactly when the platform doesn't seal");
        // The one file is the whole vault: a copy of it alone, taken while the vault is open, has every save.
        let vault = Vault::open(&dir).ok().unwrap();
        add_note(vault.conn(), "n2", "Saved a moment ago");
        let copy = folder("open-copy");
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::copy(dir.join(DB), copy.join(DB)).unwrap();
        std::fs::copy(dir.join(KEYS), copy.join(KEYS)).unwrap();
        drop(vault);
        assert_eq!(titles(Vault::open(&copy).ok().expect("the copy opens").conn()).len(), 2);
        std::fs::remove_dir_all(&copy).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn the_words_open_a_vault_this_device_cannot() {
        let dir = folder("words");
        let (words, wrong) = {
            let vault = Vault::open(&dir).ok().unwrap();
            add_note(vault.conn(), "n1", "Kept");
            (words_of(vault.key.as_ref().unwrap()).join(" "), words_of(&Zeroizing::new([1u8; 32])).join(" "))
        };
        // As on another computer, or after Windows is reinstalled: the device's copy of the key is no use.
        let mut keys = read_keys(&dir).unwrap();
        keys.device.as_mut().unwrap().wrapped = b64(b"not this account's");
        write_keys(&dir, &keys).unwrap();
        let shut = Vault::open(&dir).err().expect("the device can't open it");
        assert!(shut.key.is_none() && shut.damaged.is_none(), "locked, not damaged");
        assert!(Vault::open_with(&dir, key_from_words(&wrong).unwrap(), true).is_err());
        let vault = Vault::open_with(&dir, key_from_words(&words).unwrap(), true).ok().expect("the words open it");
        assert_eq!(titles(vault.conn()), ["Kept"]);
        drop(vault);
        assert!(Vault::open(&dir).is_ok(), "and from then on the device opens it by itself");

        // With every copy of the key file gone, the words and the database are still enough.
        for file in [dir.join(KEYS), dir.join("keys.json.bak"), dir.join(SNAPSHOTS).join(KEYS)] {
            let _ = std::fs::remove_file(file);
        }
        assert!(Vault::open(&dir).is_err());
        assert!(Vault::open_with(&dir, key_from_words(&wrong).unwrap(), true).is_err());
        let vault = Vault::open_with(&dir, key_from_words(&words).unwrap(), true).ok().expect("the words alone open it");
        assert_eq!(titles(vault.conn()), ["Kept"]);
        drop(vault);
        assert!(Vault::open(&dir).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_vault_id_is_the_same_from_the_words_and_tells_vaults_apart() {
        let key: Key = Zeroizing::new([7u8; 32]);
        let id = fingerprint(&key);
        assert_eq!(id.len(), 9);
        assert_eq!(&id[4..5], "-");
        assert_eq!(id_of_words(&words_of(&key).join(" ")).unwrap(), id, "the words give the same ID on any computer");
        assert_ne!(fingerprint(&Zeroizing::new([8u8; 32])), id);
        assert_eq!(file_id(&key), id.replace('-', ""));
    }

    #[test]
    fn backup_copies_are_found_by_vault_and_newest_first() {
        let dir = folder("backups");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["vault-7F3A90C2-100.db", "vault-7F3A90C2-300.db", "vault-0000ABCD-200.db", "vault-1700000000.db", "vault-XYZ-5.db", "keys-7F3A90C2.json", "vault-7F3A90C2-400.db.part"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let found = backups_in(&dir);
        assert_eq!(found.iter().map(|b| (b.id.as_str(), b.at)).collect::<Vec<_>>(), [("7F3A90C2", 300), ("0000ABCD", 200), ("7F3A90C2", 100)]);
        // With words given, a copy of that vault is chosen over a newer one of another.
        assert_eq!(vault_file_in(&dir, "").unwrap().file_name().unwrap(), "vault-7F3A90C2-300.db");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn another_vaults_notes_and_books_merge_in_without_touching_whats_here() {
        let here = memory();
        let there = memory();
        add_note(&here, "n1", "Mine");
        add_note(&there, "n1", "Theirs, same id");
        add_note(&there, "n2", "Only theirs");
        put_doc(&there, "chat", "n2", "", &serde_json::json!({ "messages": [] })).unwrap();
        put_doc(&there, "chat", "n1", "", &serde_json::json!({ "messages": ["not mixed in"] })).unwrap();
        put_doc(&there, "book", "b1", "", &serde_json::json!({ "title": "Their book", "notes": ["n2"] })).unwrap();
        put_doc(&there, "bookmarks", "n2", "", &serde_json::json!([{ "id": "m1" }])).unwrap();
        let mut out = BroughtIn::default();
        merge(&here, &there, &mut out).unwrap();
        assert_eq!((out.notes, out.books, out.had), (1, 1, 1));
        assert_eq!(titles(&here), ["Mine", "Only theirs"], "a note already here stands");
        assert!(get_doc::<serde_json::Value>(&here, "chat", "n1", "").is_none(), "another vault's history of a note here isn't mixed in");
        assert!(get_doc::<serde_json::Value>(&here, "bookmarks", "n2", "").is_some());
        assert_eq!(doc_names_of(&here, "book"), ["b1"]);
        // Again: nothing new.
        let mut again = BroughtIn::default();
        merge(&here, &there, &mut again).unwrap();
        assert_eq!((again.notes, again.books, again.had), (0, 0, 3));
    }

    #[test]
    fn a_backup_copy_opens_with_its_words_or_its_password_and_the_key_file_beside_it() {
        let dir = folder("key-for");
        std::fs::create_dir_all(&dir).unwrap();
        let key: Key = Zeroizing::new([5u8; 32]);
        let file = dir.join(format!("vault-{}-100.db", file_id(&key)));
        std::fs::write(&file, b"x").unwrap();
        // By the words alone, with nothing beside the copy.
        let (found, keys) = key_for(&file, &words_of(&key).join(" "), "").unwrap().unwrap();
        assert_eq!((found[..] == key[..], keys.is_none()), (true, true));
        assert!(key_for(&file, "", "a password").unwrap_err().contains("no key file"));
        assert!(key_for(&file, "", "").unwrap().is_none(), "nothing given: a copy of this vault");
        // With its key file beside it, as the daily copy has, the password opens it too.
        let mut slot = password_slot(&key, "abc", "correct horse", "").unwrap();
        (slot.m, slot.t) = (64, 1);
        let kek = stretch("correct horse", &unb64(&slot.salt).unwrap(), slot.m, slot.t, slot.p).unwrap();
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        let nonce = unb64(&slot.nonce).unwrap();
        slot.wrapped = b64(&aes_gcm::Aes256Gcm::new_from_slice(&kek[..]).unwrap().encrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &key[..], aad: b"abc" }).unwrap());
        let keys = Keys { format: 1, vault: "abc".into(), check: check_of(&key), password: Some(slot), ..Default::default() };
        std::fs::write(dir.join(format!("keys-{}.json", file_id(&key))), serde_json::to_string(&keys).unwrap()).unwrap();
        let (found, keys) = key_for(&file, "", "correct horse").unwrap().unwrap();
        assert!(found[..] == key[..] && keys.is_some_and(|k| k.vault == "abc"));
        assert!(key_for(&file, "", "wrong horse").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_database_is_told_from_a_locked_one_and_a_snapshot_reads_back() {
        let dir = folder("damage");
        let snapshot = {
            let vault = Vault::open(&dir).ok().unwrap();
            add_note(vault.conn(), "n1", "Before the damage");
            let snapshot = vault.snapshot().unwrap();
            add_note(vault.conn(), "n2", "After the copy");
            snapshot
        };
        assert_eq!(snapshots(&dir).len(), 1);
        // The copy is a database of its own, sealed like the vault.
        let key = read_keys(&dir).and_then(|k| from_device(&k));
        let copy = connect(&dir.join(SNAPSHOTS).join(&snapshot.file), key.as_ref()).unwrap();
        assert_eq!(titles(&copy), ["Before the damage"]);
        drop(copy);
        if SEALED {
            assert!(connect(&dir.join(SNAPSHOTS).join(&snapshot.file), Some(&Zeroizing::new([1u8; 32]))).is_err());
        }

        // Scribble over the middle of the database.
        let mut raw = std::fs::read(dir.join(DB)).unwrap();
        for byte in raw.iter_mut().skip(100).step_by(7) {
            *byte ^= 0x5a;
        }
        std::fs::write(dir.join(DB), raw).unwrap();
        let shut = Vault::open(&dir).err().expect("a scribbled database doesn't open");
        assert!(shut.damaged.is_some(), "damaged, not locked");
        assert_eq!(shut.key.is_some(), SEALED, "the key is still in hand to bring a copy back with");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
