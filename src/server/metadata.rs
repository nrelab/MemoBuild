use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub hash: String,
    pub artifact_path: String,
    pub size: u64,
    pub created_at: String,
    pub last_used: String,
    pub hit_count: u32,
}

pub struct MetadataStore {
    conn: Mutex<Connection>,
}

impl MetadataStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cache_entries (
                hash TEXT PRIMARY KEY,
                artifact_path TEXT,
                size BIGINT,
                created_at TIMESTAMP,
                last_used TIMESTAMP,
                hit_count INT
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, hash: &str, path: &str, size: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO cache_entries (hash, artifact_path, size, created_at, last_used, hit_count)
             VALUES (?1, ?2, ?3, ?4, ?4, 0)
             ON CONFLICT(hash) DO UPDATE SET
                last_used = ?4,
                hit_count = hit_count + 1",
            params![hash, path, size, now],
        )?;
        Ok(())
    }

    pub fn get(&self, hash: &str) -> Result<Option<CacheEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, artifact_path, size, created_at, last_used, hit_count FROM cache_entries WHERE hash = ?1",
        )?;
        let mut rows = stmt.query(params![hash])?;

        if let Some(row) = rows.next()? {
            Ok(Some(CacheEntry {
                hash: row.get(0)?,
                artifact_path: row.get(1)?,
                size: row.get(2)?,
                created_at: row.get(3)?,
                last_used: row.get(4)?,
                hit_count: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn touch(&self, hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE cache_entries SET last_used = ?1, hit_count = hit_count + 1 WHERE hash = ?2",
            params![now, hash],
        )?;
        Ok(())
    }

    pub fn exists(&self, hash: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cache_entries WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn delete(&self, hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM cache_entries WHERE hash = ?1", params![hash])?;
        Ok(())
    }

    pub fn get_old_entries(&self, days: u32) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash FROM cache_entries WHERE last_used < datetime('now', '-' || ?1 || ' days')"
        )?;
        let rows = stmt.query_map(params![days], |row| row.get(0))?;

        let mut hashes = Vec::new();
        for hash in rows {
            hashes.push(hash?);
        }
        Ok(hashes)
    }

    pub fn record_build(&self, dirty: u32, cached: u32, duration_ms: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS build_analytics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT DEFAULT CURRENT_TIMESTAMP,
                dirty_nodes INTEGER,
                cached_nodes INTEGER,
                duration_ms INTEGER
            )",
            [],
        )?;

        conn.execute(
            "INSERT INTO build_analytics (dirty_nodes, cached_nodes, duration_ms) VALUES (?1, ?2, ?3)",
            params![dirty, cached, duration_ms],
        )?;

        Ok(())
    }

    pub fn get_analytics(&self, limit: u32) -> Result<Vec<BuildRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, dirty_nodes, cached_nodes, duration_ms 
             FROM build_analytics 
             ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(BuildRecord {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                dirty_nodes: row.get(2)?,
                cached_nodes: row.get(3)?,
                duration_ms: row.get(4)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BuildRecord {
    pub id: i32,
    pub timestamp: String,
    pub dirty_nodes: u32,
    pub cached_nodes: u32,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_metadata_store() {
        let db_file = NamedTempFile::new().unwrap();
        let store = MetadataStore::new(db_file.path()).unwrap();

        let hash = "test-hash";
        let path = "some/path";
        let size = 1024;

        store.insert(hash, path, size).unwrap();
        assert!(store.exists(hash).unwrap());

        let entry = store.get(hash).unwrap().unwrap();
        assert_eq!(entry.hash, hash);
        assert_eq!(entry.artifact_path, path);
        assert_eq!(entry.size, size);

        store.touch(hash).unwrap();
        let updated_entry = store.get(hash).unwrap().unwrap();
        assert_eq!(updated_entry.hit_count, 1);
    }
}
