//! Project CRUD 操作

use rusqlite::{params, Result as SqliteResult};
use super::types::ProjectRecord;
use super::Database;

impl Database {
    /// 获取或创建项目
    pub fn get_or_create_project(&self, name: &str, root_path: &str, language: &str) -> SqliteResult<i64> {
        // 先查询是否存在
        let mut stmt = self.conn.prepare("SELECT id FROM projects WHERE root_path = ?")?;
        let result: Option<i64> = stmt.query_row([root_path], |row| row.get(0)).ok();

        if let Some(id) = result {
            return Ok(id);
        }

        // 不存在则创建
        self.conn.execute(
            "INSERT INTO projects (name, root_path, language) VALUES (?, ?, ?)",
            params![name, root_path, language],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 更新项目索引时间
    pub fn update_project_indexed_time(&self, project_id: i64) -> SqliteResult<()> {
        self.conn.execute(
            "UPDATE projects SET last_indexed_at = datetime('now') WHERE id = ?",
            [project_id],
        )?;
        Ok(())
    }

    /// 获取所有项目
    pub fn get_all_projects(&self) -> SqliteResult<Vec<ProjectRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM projects ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                language: row.get(3)?,
                last_indexed_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// 按路径获取项目
    pub fn get_project_by_path(&self, root_path: &str) -> SqliteResult<Option<ProjectRecord>> {
        let mut stmt = self.conn.prepare("SELECT * FROM projects WHERE root_path = ?")?;
        let result = stmt.query_row([root_path], |row| {
            Ok(ProjectRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                language: row.get(3)?,
                last_indexed_at: row.get(4)?,
            })
        });

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[test]
    fn test_project_crud() {
        let db = Database::open_in_memory().unwrap();

        // 创建项目
        let id1 = db.get_or_create_project("test", "/path/to/test", "rust").unwrap();
        assert!(id1 > 0);

        // 重复创建返回相同 ID
        let id2 = db.get_or_create_project("test", "/path/to/test", "rust").unwrap();
        assert_eq!(id1, id2);

        // 获取项目
        let project = db.get_project_by_path("/path/to/test").unwrap().unwrap();
        assert_eq!(project.name, "test");
        assert_eq!(project.language, "rust");

        // 更新索引时间
        db.update_project_indexed_time(id1).unwrap();
        let project = db.get_project_by_path("/path/to/test").unwrap().unwrap();
        assert!(project.last_indexed_at.is_some());

        // 获取所有项目
        let projects = db.get_all_projects().unwrap();
        assert_eq!(projects.len(), 1);
    }
}
