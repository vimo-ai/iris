//! SimilarPair CRUD 操作

use rusqlite::{params, Result as SqliteResult};
use super::types::{PairStatus, SimilarPairRecord};
use super::Database;

impl Database {
    /// 插入或更新相似配对
    pub fn upsert_similar_pair(
        &self,
        unit_a: &str,
        unit_b: &str,
        similarity: f32,
        trigger_reason: Option<&str>,
    ) -> SqliteResult<()> {
        // 保证顺序一致性
        let (a, b) = if unit_a < unit_b { (unit_a, unit_b) } else { (unit_b, unit_a) };

        self.conn.execute(
            r#"
            INSERT INTO similar_pairs (unit_a, unit_b, similarity, status, trigger_reason)
            VALUES (?, ?, ?, 'new', ?)
            ON CONFLICT(unit_a, unit_b) DO UPDATE SET
                similarity = excluded.similarity,
                trigger_reason = excluded.trigger_reason
            "#,
            params![a, b, similarity, trigger_reason],
        )?;
        Ok(())
    }

    /// 获取相似配对列表
    pub fn get_similar_pairs(
        &self,
        project_id: Option<i64>,
        status: Option<PairStatus>,
        min_similarity: f32,
    ) -> SqliteResult<Vec<SimilarPairRecord>> {
        let mut query = String::from(
            r#"
            SELECT sp.id, sp.unit_a, sp.unit_b, sp.similarity, sp.status, sp.trigger_reason,
                   ua.file_path, ua.range_start, ua.range_end,
                   ub.file_path, ub.range_start, ub.range_end
            FROM similar_pairs sp
            JOIN code_units ua ON sp.unit_a = ua.qualified_name
            JOIN code_units ub ON sp.unit_b = ub.qualified_name
            WHERE sp.similarity >= ?
            "#
        );

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_similarity)];

        if let Some(pid) = project_id {
            query.push_str(" AND ua.project_id = ?");
            params_vec.push(Box::new(pid));
        }

        if let Some(s) = status {
            query.push_str(" AND sp.status = ?");
            params_vec.push(Box::new(s.as_str().to_string()));
        }

        query.push_str(" ORDER BY sp.similarity DESC");

        let mut stmt = self.conn.prepare(&query)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let status_str: String = row.get(4)?;
            Ok(SimilarPairRecord {
                id: row.get(0)?,
                unit_a: row.get(1)?,
                unit_b: row.get(2)?,
                similarity: row.get(3)?,
                status: PairStatus::from_str(&status_str).unwrap_or(PairStatus::New),
                trigger_reason: row.get(5)?,
                file_a: row.get(6)?,
                start_a: row.get(7)?,
                end_a: row.get(8)?,
                file_b: row.get(9)?,
                start_b: row.get(10)?,
                end_b: row.get(11)?,
            })
        })?;
        rows.collect()
    }

    /// 更新配对状态
    pub fn update_pair_status(&self, pair_id: i64, status: PairStatus) -> SqliteResult<()> {
        self.conn.execute(
            "UPDATE similar_pairs SET status = ? WHERE id = ?",
            params![status.as_str(), pair_id],
        )?;
        Ok(())
    }

    /// 删除涉及某 CodeUnit 的所有配对
    pub fn delete_pairs_involving(&self, qualified_name: &str) -> SqliteResult<()> {
        self.conn.execute(
            "DELETE FROM similar_pairs WHERE unit_a = ? OR unit_b = ?",
            params![qualified_name, qualified_name],
        )?;
        Ok(())
    }

    /// 批量插入相似配对（单事务，高效）
    pub fn batch_upsert_similar_pairs(
        &self,
        pairs: &[(String, String, f32)],
        trigger_reason: Option<&str>,
    ) -> SqliteResult<usize> {
        // 显式开启事务
        self.conn.execute("BEGIN TRANSACTION", [])?;

        let result = (|| {
            let mut stmt = self.conn.prepare(
                r#"
                INSERT INTO similar_pairs (unit_a, unit_b, similarity, status, trigger_reason)
                VALUES (?, ?, ?, 'new', ?)
                ON CONFLICT(unit_a, unit_b) DO UPDATE SET
                    similarity = excluded.similarity,
                    trigger_reason = excluded.trigger_reason
                "#,
            )?;

            let mut count = 0;
            for (unit_a, unit_b, similarity) in pairs {
                // 保证顺序一致性
                let (a, b) = if unit_a < unit_b { (unit_a.as_str(), unit_b.as_str()) } else { (unit_b.as_str(), unit_a.as_str()) };
                stmt.execute(params![a, b, similarity, trigger_reason])?;
                count += 1;
            }

            Ok::<usize, rusqlite::Error>(count)
        })();

        match result {
            Ok(count) => {
                self.conn.execute("COMMIT", [])?;
                Ok(count)
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, CodeUnitRecord, PairStatus};

    fn setup_db_with_units() -> (Database, i64) {
        let db = Database::open_in_memory().unwrap();
        let project_id = db.get_or_create_project("test", "/path", "rust").unwrap();

        for name in ["rust::a", "rust::b"] {
            let record = CodeUnitRecord {
                qualified_name: name.to_string(),
                project_id,
                file_path: "/path/src/lib.rs".to_string(),
                kind: "function".to_string(),
                range_start: 10,
                range_end: 20,
                content_hash: format!("hash_{}", name),
                structure_hash: format!("struct_{}", name),
                embedding: None,
                group_id: None,
            };
            db.upsert_code_unit(&record).unwrap();
        }

        (db, project_id)
    }

    #[test]
    fn test_similar_pair_crud() {
        let (db, _) = setup_db_with_units();

        // 插入配对
        db.upsert_similar_pair("rust::a", "rust::b", 0.95, Some("test")).unwrap();

        // 查询
        let pairs = db.get_similar_pairs(None, None, 0.0).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].similarity, 0.95);
        assert_eq!(pairs[0].status, PairStatus::New);

        // 更新状态
        db.update_pair_status(pairs[0].id, PairStatus::Confirmed).unwrap();
        let pairs = db.get_similar_pairs(None, Some(PairStatus::Confirmed), 0.0).unwrap();
        assert_eq!(pairs.len(), 1);

        // 过滤相似度
        let pairs = db.get_similar_pairs(None, None, 0.99).unwrap();
        assert_eq!(pairs.len(), 0);

        // 删除
        db.delete_pairs_involving("rust::a").unwrap();
        let pairs = db.get_similar_pairs(None, None, 0.0).unwrap();
        assert_eq!(pairs.len(), 0);
    }

    #[test]
    fn test_pair_ordering_consistency() {
        let (db, _) = setup_db_with_units();

        // 无论顺序如何，都应该更新同一条记录
        db.upsert_similar_pair("rust::b", "rust::a", 0.90, None).unwrap();
        db.upsert_similar_pair("rust::a", "rust::b", 0.95, None).unwrap();

        let pairs = db.get_similar_pairs(None, None, 0.0).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].similarity, 0.95); // 更新后的值
    }
}
