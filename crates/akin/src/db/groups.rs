//! Group 与 Stats 操作

use rusqlite::{params, Result as SqliteResult};
use std::collections::HashMap;
use super::types::{SimilarityGroupRecord, ProjectStats};
use super::Database;

impl Database {
    /// 创建分组
    pub fn create_group(
        &self,
        project_id: i64,
        name: &str,
        reason: Option<&str>,
        pattern: Option<&str>,
    ) -> SqliteResult<i64> {
        self.conn.execute(
            "INSERT INTO similarity_groups (project_id, name, reason, pattern) VALUES (?, ?, ?, ?)",
            params![project_id, name, reason, pattern],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 将 CodeUnit 添加到分组
    pub fn add_to_group(&self, qualified_name: &str, group_id: i64) -> SqliteResult<()> {
        self.conn.execute(
            "UPDATE code_units SET group_id = ? WHERE qualified_name = ?",
            params![group_id, qualified_name],
        )?;
        Ok(())
    }

    /// 获取项目的所有分组
    pub fn get_groups(&self, project_id: i64) -> SqliteResult<Vec<SimilarityGroupRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM similarity_groups WHERE project_id = ?")?;
        let rows = stmt.query_map([project_id], |row| {
            Ok(SimilarityGroupRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                reason: row.get(3)?,
                pattern: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// 获取项目统计信息
    pub fn get_stats(&self, project_id: i64) -> SqliteResult<ProjectStats> {
        let total_units: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM code_units WHERE project_id = ?",
            [project_id],
            |row| row.get(0),
        )?;

        let mut pairs_by_status = HashMap::new();
        let mut stmt = self.conn.prepare(
            r#"
            SELECT sp.status, COUNT(*) as count
            FROM similar_pairs sp
            JOIN code_units u ON sp.unit_a = u.qualified_name
            WHERE u.project_id = ?
            GROUP BY sp.status
            "#
        )?;
        let rows = stmt.query_map([project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (status, count) = row?;
            pairs_by_status.insert(status, count);
        }

        let total_groups: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM similarity_groups WHERE project_id = ?",
            [project_id],
            |row| row.get(0),
        )?;

        Ok(ProjectStats {
            total_units,
            pairs_by_status,
            total_groups,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, CodeUnitRecord};

    #[test]
    fn test_similarity_group() {
        let db = Database::open_in_memory().unwrap();
        let project_id = db.get_or_create_project("test", "/path", "rust").unwrap();

        // 创建分组
        let group_id = db.create_group(project_id, "Error handlers", Some("相似的错误处理"), None).unwrap();
        assert!(group_id > 0);

        // 创建 code unit 并添加到分组
        let record = CodeUnitRecord {
            qualified_name: "rust::test::foo".to_string(),
            project_id,
            file_path: "/path/src/lib.rs".to_string(),
            kind: "function".to_string(),
            range_start: 10,
            range_end: 20,
            content_hash: "abc".to_string(),
            structure_hash: "def".to_string(),
            embedding: None,
            group_id: None,
        };
        db.upsert_code_unit(&record).unwrap();
        db.add_to_group("rust::test::foo", group_id).unwrap();

        // 验证
        let unit = db.get_code_unit("rust::test::foo").unwrap().unwrap();
        assert_eq!(unit.group_id, Some(group_id));

        // 获取分组列表
        let groups = db.get_groups(project_id).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Error handlers");
    }

    #[test]
    fn test_stats() {
        let db = Database::open_in_memory().unwrap();
        let project_id = db.get_or_create_project("test", "/path", "rust").unwrap();

        // 创建 code units
        for i in 0..3 {
            let record = CodeUnitRecord {
                qualified_name: format!("rust::func_{}", i),
                project_id,
                file_path: "/path/src/lib.rs".to_string(),
                kind: "function".to_string(),
                range_start: i * 10,
                range_end: (i + 1) * 10,
                content_hash: format!("hash_{}", i),
                structure_hash: format!("struct_{}", i),
                embedding: None,
                group_id: None,
            };
            db.upsert_code_unit(&record).unwrap();
        }

        // 创建配对
        db.upsert_similar_pair("rust::func_0", "rust::func_1", 0.90, None).unwrap();

        // 创建分组
        db.create_group(project_id, "test_group", None, None).unwrap();

        // 获取统计
        let stats = db.get_stats(project_id).unwrap();
        assert_eq!(stats.total_units, 3);
        assert_eq!(stats.total_groups, 1);
        assert_eq!(stats.pairs_by_status.get("new"), Some(&1));
    }
}
