//! CodeUnit CRUD 操作

use rusqlite::{params, Result as SqliteResult};
use super::types::CodeUnitRecord;
use super::Database;

impl Database {
    /// 插入或更新 CodeUnit
    pub fn upsert_code_unit(&self, record: &CodeUnitRecord) -> SqliteResult<()> {
        // 查找是否有相同 structure_hash 的记录可以继承 group_id
        let inherited_group_id: Option<i64> = self.conn
            .prepare("SELECT group_id FROM code_units WHERE structure_hash = ? AND qualified_name != ?")?
            .query_row(params![&record.structure_hash, &record.qualified_name], |row| row.get(0))
            .ok()
            .flatten();

        self.conn.execute(
            r#"
            INSERT INTO code_units
                (qualified_name, project_id, file_path, kind, range_start, range_end,
                 content_hash, structure_hash, embedding, group_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(qualified_name) DO UPDATE SET
                file_path = excluded.file_path,
                kind = excluded.kind,
                range_start = excluded.range_start,
                range_end = excluded.range_end,
                content_hash = excluded.content_hash,
                structure_hash = excluded.structure_hash,
                embedding = COALESCE(excluded.embedding, code_units.embedding),
                group_id = COALESCE(code_units.group_id, excluded.group_id)
            "#,
            params![
                &record.qualified_name,
                record.project_id,
                &record.file_path,
                &record.kind,
                record.range_start,
                record.range_end,
                &record.content_hash,
                &record.structure_hash,
                &record.embedding,
                inherited_group_id.or(record.group_id),
            ],
        )?;
        Ok(())
    }

    /// 获取单个 CodeUnit
    pub fn get_code_unit(&self, qualified_name: &str) -> SqliteResult<Option<CodeUnitRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM code_units WHERE qualified_name = ?")?;
        let result = stmt.query_row([qualified_name], Self::row_to_code_unit);

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 获取项目的所有 CodeUnits
    pub fn get_code_units_by_project(&self, project_id: i64) -> SqliteResult<Vec<CodeUnitRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM code_units WHERE project_id = ?")?;
        let rows = stmt.query_map([project_id], Self::row_to_code_unit)?;
        rows.collect()
    }

    /// 获取多个项目的 CodeUnits (None 表示全部)
    pub fn get_code_units_by_projects(&self, project_ids: Option<&[i64]>) -> SqliteResult<Vec<CodeUnitRecord>> {
        match project_ids {
            None => {
                let mut stmt = self.conn.prepare("SELECT * FROM code_units")?;
                let rows = stmt.query_map([], Self::row_to_code_unit)?;
                rows.collect()
            }
            Some(ids) if ids.is_empty() => Ok(vec![]),
            Some(ids) => {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let query = format!("SELECT * FROM code_units WHERE project_id IN ({})", placeholders);
                let mut stmt = self.conn.prepare(&query)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), Self::row_to_code_unit)?;
                rows.collect()
            }
        }
    }

    /// 获取文件的所有 CodeUnits
    pub fn get_code_units_by_file(&self, file_path: &str) -> SqliteResult<Vec<CodeUnitRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM code_units WHERE file_path = ?")?;
        let rows = stmt.query_map([file_path], Self::row_to_code_unit)?;
        rows.collect()
    }

    /// 删除文件的所有 CodeUnits
    pub fn delete_code_units_by_file(&self, file_path: &str) -> SqliteResult<()> {
        self.conn.execute("DELETE FROM code_units WHERE file_path = ?", [file_path])?;
        Ok(())
    }

    /// 按 content_hash 获取已缓存的 embedding
    pub fn get_embedding_by_content_hash(&self, content_hash: &str) -> SqliteResult<Option<Vec<u8>>> {
        let mut stmt = self.conn.prepare(
            "SELECT embedding FROM code_units WHERE content_hash = ? AND embedding IS NOT NULL LIMIT 1"
        )?;
        let result: Result<Vec<u8>, _> = stmt.query_row([content_hash], |row| row.get(0));

        match result {
            Ok(emb) => Ok(Some(emb)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(super) fn row_to_code_unit(row: &rusqlite::Row) -> rusqlite::Result<CodeUnitRecord> {
        Ok(CodeUnitRecord {
            qualified_name: row.get(0)?,
            project_id: row.get(1)?,
            file_path: row.get(2)?,
            kind: row.get(3)?,
            range_start: row.get(4)?,
            range_end: row.get(5)?,
            content_hash: row.get(6)?,
            structure_hash: row.get(7)?,
            embedding: row.get(8)?,
            group_id: row.get(9)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, CodeUnitRecord};

    #[test]
    fn test_code_unit_crud() {
        let db = Database::open_in_memory().unwrap();
        let project_id = db.get_or_create_project("test", "/path", "rust").unwrap();

        let record = CodeUnitRecord {
            qualified_name: "rust::test::foo".to_string(),
            project_id,
            file_path: "/path/src/lib.rs".to_string(),
            kind: "function".to_string(),
            range_start: 10,
            range_end: 20,
            content_hash: "abc123".to_string(),
            structure_hash: "def456".to_string(),
            embedding: Some(vec![1, 2, 3, 4]),
            group_id: None,
        };

        // 插入
        db.upsert_code_unit(&record).unwrap();

        // 查询
        let loaded = db.get_code_unit("rust::test::foo").unwrap().unwrap();
        assert_eq!(loaded.file_path, "/path/src/lib.rs");
        assert_eq!(loaded.embedding, Some(vec![1, 2, 3, 4]));

        // 更新
        let updated = CodeUnitRecord {
            range_end: 25,
            ..record.clone()
        };
        db.upsert_code_unit(&updated).unwrap();
        let loaded = db.get_code_unit("rust::test::foo").unwrap().unwrap();
        assert_eq!(loaded.range_end, 25);

        // 按项目查询
        let units = db.get_code_units_by_project(project_id).unwrap();
        assert_eq!(units.len(), 1);

        // 按文件查询
        let units = db.get_code_units_by_file("/path/src/lib.rs").unwrap();
        assert_eq!(units.len(), 1);

        // 删除
        db.delete_code_units_by_file("/path/src/lib.rs").unwrap();
        let units = db.get_code_units_by_project(project_id).unwrap();
        assert_eq!(units.len(), 0);
    }

    #[test]
    fn test_embedding_cache() {
        let db = Database::open_in_memory().unwrap();
        let project_id = db.get_or_create_project("test", "/path", "rust").unwrap();

        let record = CodeUnitRecord {
            qualified_name: "rust::test::foo".to_string(),
            project_id,
            file_path: "/path/src/lib.rs".to_string(),
            kind: "function".to_string(),
            range_start: 10,
            range_end: 20,
            content_hash: "same_hash".to_string(),
            structure_hash: "struct_hash".to_string(),
            embedding: Some(vec![1, 2, 3, 4]),
            group_id: None,
        };
        db.upsert_code_unit(&record).unwrap();

        // 相同 content_hash 可以复用 embedding
        let cached = db.get_embedding_by_content_hash("same_hash").unwrap();
        assert_eq!(cached, Some(vec![1, 2, 3, 4]));

        // 不存在的 hash 返回 None
        let none = db.get_embedding_by_content_hash("other_hash").unwrap();
        assert!(none.is_none());
    }
}
