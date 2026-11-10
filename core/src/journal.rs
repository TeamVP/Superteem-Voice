//! Append-only per-source audio journal, readable after a crash.
//!
//! File: "STVJ", version u8. Record: length u32, crc32 u32, arrival_us u64, frame bytes.
//! The length and crc cover arrival_us and the frame.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"STVJ";
const VERSION: u8 = 1;
const FILE_HEADER: usize = 5;
const MAX_RECORD: usize = 64 * 1024;

pub struct JournalWriter {
    out: BufWriter<File>,
}

impl JournalWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(MAGIC)?;
        file.write_all(&[VERSION])?;
        file.sync_all()?;
        Ok(Self { out: BufWriter::new(file) })
    }

    pub fn append(&mut self, arrival_us: u64, frame: &[u8]) -> io::Result<()> {
        if frame.len() + 8 > MAX_RECORD {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
        }
        let arrival = arrival_us.to_le_bytes();
        let mut crc = crc32fast::Hasher::new();
        crc.update(&arrival);
        crc.update(frame);
        self.out.write_all(&((frame.len() + 8) as u32).to_le_bytes())?;
        self.out.write_all(&crc.finalize().to_le_bytes())?;
        self.out.write_all(&arrival)?;
        self.out.write_all(frame)
    }

    /// Makes everything appended so far durable. The capture service calls this at least once a second.
    pub fn sync(&mut self) -> io::Result<()> {
        self.out.flush()?;
        self.out.get_ref().sync_data()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub arrival_us: u64,
    pub frame: Vec<u8>,
}

#[derive(Debug)]
pub struct Recovered {
    pub records: Vec<Record>,
    /// Length of the valid prefix. Anything after it is a torn or corrupt tail.
    pub valid_bytes: u64,
    pub had_bad_tail: bool,
}

/// Reads every intact record, stopping at the first torn or corrupt one.
pub fn read(path: &Path) -> io::Result<Recovered> {
    let data = fs::read(path)?;
    if data.len() < FILE_HEADER || &data[..4] != MAGIC || data[4] != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a journal file"));
    }
    let mut records = Vec::new();
    let mut pos = FILE_HEADER;
    while let Some(head) = data.get(pos..pos + 8) {
        let len = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(head[4..].try_into().unwrap());
        if !(8..=MAX_RECORD).contains(&len) {
            break;
        }
        let Some(body) = data.get(pos + 8..pos + 8 + len) else { break };
        if crc32fast::hash(body) != crc {
            break;
        }
        records.push(Record {
            arrival_us: u64::from_le_bytes(body[..8].try_into().unwrap()),
            frame: body[8..].to_vec(),
        });
        pos += 8 + len;
    }
    Ok(Recovered { records, valid_bytes: pos as u64, had_bad_tail: pos != data.len() })
}

/// Reads the records written since byte `from` of a journal that is still being written, for following a
/// recording as it happens. Returns them and where the next read starts; a record still being written is
/// left for next time. `from` is 0 the first time.
pub fn read_since(path: &Path, from: u64) -> io::Result<(Vec<Record>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let start = if from == 0 {
        let mut head = [0u8; FILE_HEADER];
        if file.read_exact(&mut head).is_err() {
            return Ok((Vec::new(), 0));
        }
        if &head[..4] != MAGIC || head[4] != VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a journal file"));
        }
        FILE_HEADER as u64
    } else {
        from
    };
    file.seek(SeekFrom::Start(start))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    let mut records = Vec::new();
    let mut pos = 0usize;
    while let Some(head) = data.get(pos..pos + 8) {
        let len = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(head[4..].try_into().unwrap());
        if !(8..=MAX_RECORD).contains(&len) {
            break;
        }
        let Some(body) = data.get(pos + 8..pos + 8 + len) else { break };
        if crc32fast::hash(body) != crc {
            break;
        }
        records.push(Record { arrival_us: u64::from_le_bytes(body[..8].try_into().unwrap()), frame: body[8..].to_vec() });
        pos += 8 + len;
    }
    Ok((records, start + pos as u64))
}

/// Removes a torn tail, for finalising a session interrupted by a crash.
pub fn repair(path: &Path) -> io::Result<Recovered> {
    let recovered = read(path)?;
    if recovered.had_bad_tail {
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_len(recovered.valid_bytes)?;
        file.sync_all()?;
    }
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("stv-{}-{name}", std::process::id()));
        let _ = fs::remove_file(&path);
        path
    }

    fn write_three(path: &Path) {
        let mut w = JournalWriter::create(path).unwrap();
        for i in 0..3u8 {
            w.append(i as u64, &[i; 10]).unwrap();
        }
        w.sync().unwrap();
    }

    #[test]
    fn every_truncation_recovers_a_valid_prefix() {
        let path = temp("trunc");
        write_three(&path);
        let full = fs::read(&path).unwrap();
        for cut in FILE_HEADER..full.len() {
            fs::write(&path, &full[..cut]).unwrap();
            let got = read(&path).unwrap();
            assert_eq!(got.records.len(), (cut - FILE_HEADER) / 26);
            for (i, r) in got.records.iter().enumerate() {
                assert_eq!(r.frame, vec![i as u8; 10]);
            }
        }
    }

    #[test]
    fn corruption_stops_at_the_bad_record_and_repair_removes_it() {
        let path = temp("corrupt");
        write_three(&path);
        let mut data = fs::read(&path).unwrap();
        data[FILE_HEADER + 26 + 20] ^= 0xFF;
        fs::write(&path, &data).unwrap();
        let got = repair(&path).unwrap();
        assert_eq!(got.records.len(), 1);
        assert!(got.had_bad_tail);
        assert!(!read(&path).unwrap().had_bad_tail);
    }

    #[test]
    fn following_a_journal_reads_each_record_once() {
        let path = temp("follow");
        write_three(&path);
        let full = fs::read(&path).unwrap();
        // Half-way through the second record: only the first is read, and the second comes next time.
        fs::write(&path, &full[..FILE_HEADER + 26 + 10]).unwrap();
        let (first, at) = read_since(&path, 0).unwrap();
        assert_eq!(first.len(), 1);
        fs::write(&path, &full).unwrap();
        let (rest, end) = read_since(&path, at).unwrap();
        assert_eq!(rest.iter().map(|r| r.frame[0]).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(end, full.len() as u64);
        assert!(read_since(&path, end).unwrap().0.is_empty());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_journal() {
        let path = temp("exists");
        write_three(&path);
        assert!(JournalWriter::create(&path).is_err());
    }
}
