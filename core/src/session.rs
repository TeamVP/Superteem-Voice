//! Session database. Schema follows PRD section 10.

use rusqlite::{params, Connection, Result};
use std::path::Path;

const SCHEMA_V1: &str = r#"
CREATE TABLE sessions (
    id TEXT PRIMARY KEY, title TEXT, started_at TEXT NOT NULL, ended_at TEXT,
    status TEXT NOT NULL, consent_confirmed_at TEXT, pipeline_version TEXT
);
CREATE TABLE devices (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT, hardware_id TEXT,
    firmware_version TEXT, capabilities_json TEXT
);
CREATE TABLE speakers (
    id TEXT PRIMARY KEY, display_name TEXT, is_guest INTEGER NOT NULL DEFAULT 0,
    voiceprint_path TEXT, consent_at TEXT
);
CREATE TABLE media_assets (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    device_id TEXT REFERENCES devices(id), file_path TEXT NOT NULL, codec TEXT NOT NULL,
    sample_rate INTEGER NOT NULL, duration_samples INTEGER, origin TEXT NOT NULL
);
CREATE TABLE tracks (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    device_id TEXT REFERENCES devices(id), speaker_id TEXT REFERENCES speakers(id) ON DELETE SET NULL,
    name TEXT NOT NULL, track_index INTEGER NOT NULL
);
CREATE TABLE timeline_clips (
    id TEXT PRIMARY KEY, track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL REFERENCES media_assets(id) ON DELETE CASCADE,
    timeline_start_samples INTEGER NOT NULL, timeline_end_samples INTEGER NOT NULL,
    source_in_samples INTEGER NOT NULL, source_out_samples INTEGER NOT NULL,
    drift_ppm REAL NOT NULL DEFAULT 0.0, base_gain_db REAL NOT NULL DEFAULT 0.0
);
CREATE TABLE gain_ranges (
    id INTEGER PRIMARY KEY AUTOINCREMENT, track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    start_samples INTEGER NOT NULL, end_samples INTEGER NOT NULL, gain_db REAL NOT NULL, source TEXT NOT NULL
);
CREATE TABLE gaps (
    id INTEGER PRIMARY KEY AUTOINCREMENT, track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    start_samples INTEGER NOT NULL, end_samples INTEGER NOT NULL, reason TEXT,
    backfilled INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE transcript_segments (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
    speaker_id TEXT REFERENCES speakers(id) ON DELETE SET NULL,
    start_samples INTEGER NOT NULL, end_samples INTEGER NOT NULL, text TEXT NOT NULL, words_json TEXT,
    confidence REAL, model_id TEXT, user_edited INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE bookmarks (
    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    device_id TEXT REFERENCES devices(id), position_samples INTEGER NOT NULL, label TEXT
);
"#;

const MIGRATIONS: &[&str] = &[SCHEMA_V1];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Recording,
    Interrupted,
    Processing,
    Ready,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Recording => "recording",
            Status::Interrupted => "interrupted",
            Status::Processing => "processing",
            Status::Ready => "ready",
        }
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::setup(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(Self { conn })
    }

    pub fn create_session(&self, id: &str, title: &str, started_at: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, title, started_at, status) VALUES (?1, ?2, ?3, 'recording')",
            params![id, title, started_at],
        )?;
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: Status) -> Result<()> {
        self.conn.execute("UPDATE sessions SET status = ?2 WHERE id = ?1", params![id, status.as_str()])?;
        Ok(())
    }

    pub fn status(&self, id: &str) -> Result<String> {
        self.conn.query_row("SELECT status FROM sessions WHERE id = ?1", [id], |r| r.get(0))
    }

    pub fn confirm_consent(&self, id: &str, at: &str) -> Result<()> {
        self.conn.execute("UPDATE sessions SET consent_confirmed_at = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    /// Sessions still marked recording when the app starts were cut off by a crash.
    pub fn mark_interrupted_on_launch(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM sessions WHERE status = 'recording'")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>>>()?;
        self.conn.execute("UPDATE sessions SET status = 'interrupted' WHERE status = 'recording'", [])?;
        Ok(ids)
    }

    pub fn add_speaker(&self, id: &str, name: &str, is_guest: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO speakers (id, display_name, is_guest) VALUES (?1, ?2, ?3)",
            params![id, name, is_guest],
        )?;
        Ok(())
    }

    pub fn insert_machine_segment(
        &self,
        id: &str,
        session: &str,
        speaker: Option<&str>,
        start: i64,
        end: i64,
        text: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO transcript_segments (id, session_id, speaker_id, start_samples, end_samples, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session, speaker, start, end, text],
        )?;
        Ok(())
    }

    pub fn edit_segment(&self, id: &str, text: &str) -> Result<()> {
        self.conn.execute("UPDATE transcript_segments SET text = ?2, user_edited = 1 WHERE id = ?1", params![id, text])?;
        Ok(())
    }

    /// Clears machine output before reprocessing. User corrections are never removed.
    pub fn clear_machine_segments(&self, session: &str) -> Result<usize> {
        self.conn.execute("DELETE FROM transcript_segments WHERE session_id = ?1 AND user_edited = 0", [session])
    }

    pub fn segment_texts(&self, session: &str) -> Result<Vec<String>> {
        let mut stmt =
            self.conn.prepare("SELECT text FROM transcript_segments WHERE session_id = ?1 ORDER BY start_samples")?;
        let rows = stmt.query_map([session], |r| r.get(0))?;
        rows.collect()
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Removes a speaker. Their segments stay but lose the name.
    pub fn delete_speaker(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM speakers WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn count(&self, table: &str) -> Result<i64> {
        self.conn.query_row(&format!("SELECT COUNT(*) FROM \"{}\"", table.replace('"', "")), [], |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        let db = Db::in_memory().unwrap();
        db.create_session("s1", "Standup", "2026-11-07T00:00:00Z").unwrap();
        db
    }

    #[test]
    fn crash_recovery_finds_unfinished_sessions() {
        let db = db();
        assert_eq!(db.mark_interrupted_on_launch().unwrap(), vec!["s1"]);
        assert_eq!(db.status("s1").unwrap(), "interrupted");
        assert!(db.mark_interrupted_on_launch().unwrap().is_empty());
    }

    #[test]
    fn reprocessing_preserves_user_edits() {
        let db = db();
        db.insert_machine_segment("a", "s1", None, 0, 10, "machine").unwrap();
        db.insert_machine_segment("b", "s1", None, 10, 20, "wrong").unwrap();
        db.edit_segment("b", "corrected").unwrap();
        assert_eq!(db.clear_machine_segments("s1").unwrap(), 1);
        assert_eq!(db.segment_texts("s1").unwrap(), vec!["corrected"]);
    }

    #[test]
    fn deleting_a_session_removes_its_data() {
        let db = db();
        db.insert_machine_segment("a", "s1", None, 0, 10, "text").unwrap();
        db.delete_session("s1").unwrap();
        assert_eq!(db.count("transcript_segments").unwrap(), 0);
    }

    #[test]
    fn deleting_a_speaker_keeps_the_text_but_removes_the_name() {
        let db = db();
        db.add_speaker("alice", "Alice", false).unwrap();
        db.insert_machine_segment("a", "s1", Some("alice"), 0, 10, "text").unwrap();
        db.delete_speaker("alice").unwrap();
        assert_eq!(db.segment_texts("s1").unwrap(), vec!["text"]);
        assert_eq!(db.count("speakers").unwrap(), 0);
    }

    #[test]
    fn migration_is_safe_to_run_twice() {
        let path = std::env::temp_dir().join(format!("stv-{}-db.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Db::open(&path).unwrap().create_session("s", "t", "now").unwrap();
        assert_eq!(Db::open(&path).unwrap().status("s").unwrap(), "recording");
        let _ = std::fs::remove_file(&path);
    }
}
