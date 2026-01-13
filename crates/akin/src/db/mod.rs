//! 数据库模块 - 管理代码单元、相似配对和分组

mod types;
mod project;
mod code_unit;
mod pairs;
mod groups;

pub use types::*;

use rusqlite::{Connection, Result as SqliteResult};
use std::path::Path;

/// 数据库管理
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> SqliteResult<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> SqliteResult<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> SqliteResult<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                root_path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                last_indexed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS code_units (
                qualified_name TEXT PRIMARY KEY,
                project_id INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                kind TEXT NOT NULL,
                range_start INTEGER NOT NULL,
                range_end INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                structure_hash TEXT NOT NULL,
                embedding BLOB,
                group_id INTEGER,
                FOREIGN KEY (project_id) REFERENCES projects(id)
            );

            CREATE TABLE IF NOT EXISTS similar_pairs (
                id INTEGER PRIMARY KEY,
                unit_a TEXT NOT NULL,
                unit_b TEXT NOT NULL,
                similarity REAL NOT NULL,
                status TEXT NOT NULL DEFAULT 'new',
                trigger_reason TEXT,
                FOREIGN KEY (unit_a) REFERENCES code_units(qualified_name),
                FOREIGN KEY (unit_b) REFERENCES code_units(qualified_name),
                UNIQUE(unit_a, unit_b)
            );

            CREATE TABLE IF NOT EXISTS similarity_groups (
                id INTEGER PRIMARY KEY,
                project_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                reason TEXT,
                pattern TEXT,
                FOREIGN KEY (project_id) REFERENCES projects(id)
            );

            CREATE INDEX IF NOT EXISTS idx_units_project ON code_units(project_id);
            CREATE INDEX IF NOT EXISTS idx_units_hash ON code_units(content_hash);
            CREATE INDEX IF NOT EXISTS idx_pairs_status ON similar_pairs(status);
            "#,
        )?;
        Ok(())
    }
}
