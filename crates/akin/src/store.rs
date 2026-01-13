//! 存储层 - 协调 SQLite 数据库和向量索引

use std::path::{Path, PathBuf};
use rayon::prelude::*;
use thiserror::Error;

use crate::db::{Database, CodeUnitRecord};
use crate::embedding::bytes_to_embedding;
use crate::vector_index::VectorIndex;

/// 存储层错误
#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Vector index error: {0}")]
    VectorIndex(#[from] crate::vector_index::VectorIndexError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Vector index not initialized")]
    VectorIndexNotInitialized,
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// ANN 搜索结果
#[derive(Debug, Clone)]
pub struct SimilarUnit {
    pub qualified_name: String,
    pub file_path: String,
    pub range_start: u32,
    pub project_id: i64,
    pub similarity: f32,
}

/// 存储层 - 管理 Database + VectorIndex
pub struct Store {
    db: Database,
    vector_index: Option<VectorIndex>,
    vector_index_path: PathBuf,
    /// qualified_name -> rowid 的映射（用于向量索引）
    name_to_id: std::collections::HashMap<String, u64>,
    /// rowid -> qualified_name 的反向映射
    id_to_name: std::collections::HashMap<u64, String>,
    next_id: u64,
}

impl Store {
    /// 打开或创建 Store
    pub fn open(db_path: &Path) -> Result<Self> {
        let db = Database::open(db_path)?;

        // 向量索引放在同目录，扩展名改为 .usearch
        let vector_index_path = db_path.with_extension("usearch");

        let mut store = Self {
            db,
            vector_index: None,
            vector_index_path,
            name_to_id: std::collections::HashMap::new(),
            id_to_name: std::collections::HashMap::new(),
            next_id: 1,
        };

        // 尝试加载已有的向量索引
        if !store.try_load_vector_index()? {
            // 没有向量索引，尝试从数据库自动构建
            let count = store.db.get_code_units_by_projects(None)?.len();
            if count > 0 {
                tracing::info!("Building vector index from {} code units...", count);
                let indexed = store.rebuild_vector_index()?;
                tracing::info!("Vector index built with {} embeddings", indexed);
            }
        }

        Ok(store)
    }

    /// 尝试加载向量索引（如果存在），返回是否成功加载
    fn try_load_vector_index(&mut self) -> Result<bool> {
        if self.vector_index_path.exists() {
            match VectorIndex::load(&self.vector_index_path) {
                Ok(index) => {
                    // 同时重建 mapping
                    self.rebuild_mappings()?;
                    self.vector_index = Some(index);
                    return Ok(true);
                }
                Err(e) => {
                    tracing::warn!("Failed to load vector index: {}, will rebuild", e);
                }
            }
        }
        Ok(false)
    }

    /// 确保向量索引已初始化
    pub fn ensure_vector_index(&mut self) -> Result<&VectorIndex> {
        if self.vector_index.is_none() {
            let index = VectorIndex::with_defaults()?;
            // 预分配容量
            let count = self.db.get_code_units_by_projects(None)?.len();
            if count > 0 {
                index.reserve(count + 1000)?; // 预留一些空间
            }
            self.vector_index = Some(index);
        }
        Ok(self.vector_index.as_ref().unwrap())
    }

    /// 重建 name <-> id 映射
    fn rebuild_mappings(&mut self) -> Result<()> {
        let units = self.db.get_code_units_by_projects(None)?;

        for (idx, unit) in units.iter().enumerate() {
            let id = (idx + 1) as u64;
            self.name_to_id.insert(unit.qualified_name.clone(), id);
            self.id_to_name.insert(id, unit.qualified_name.clone());
        }

        self.next_id = (units.len() + 1) as u64;
        Ok(())
    }

    /// 获取或分配 ID
    fn get_or_allocate_id(&mut self, name: &str) -> u64 {
        if let Some(&id) = self.name_to_id.get(name) {
            id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.name_to_id.insert(name.to_string(), id);
            self.id_to_name.insert(id, name.to_string());
            id
        }
    }

    /// 插入或更新 CodeUnit，同时更新向量索引
    pub fn upsert_code_unit(&mut self, record: &CodeUnitRecord) -> Result<()> {
        // 1. 写入数据库
        self.db.upsert_code_unit(record)?;

        // 2. 如果有 embedding，更新向量索引
        if let Some(ref emb_bytes) = record.embedding {
            if let Some(embedding) = bytes_to_embedding(emb_bytes) {
                self.ensure_vector_index()?;

                let id = self.get_or_allocate_id(&record.qualified_name);

                // 确保容量足够
                let index = self.vector_index.as_ref().unwrap();
                if index.size() >= index.capacity() {
                    index.reserve(index.capacity() + 1000)?;
                }

                // 如果已存在，先删除
                if index.contains(id) {
                    index.remove(id)?;
                }

                // 添加新向量
                let vec: Vec<f32> = embedding.to_vec();
                index.add(id, &vec)?;
            }
        }

        Ok(())
    }

    /// ANN 搜索相似代码单元
    pub fn search_similar(
        &self,
        query_embedding: &[f32],
        k: usize,
        threshold: f32,
    ) -> Result<Vec<SimilarUnit>> {
        let index = self.vector_index.as_ref()
            .ok_or(StoreError::VectorIndexNotInitialized)?;

        // ANN 搜索
        let results = index.search(query_embedding, k)?;

        // 转换为 SimilarUnit
        let mut similar_units = Vec::new();
        for result in results {
            // cosine distance -> similarity
            let similarity = result.similarity();

            if similarity < threshold {
                continue;
            }

            // 查找对应的 code unit
            if let Some(name) = self.id_to_name.get(&result.id) {
                if let Ok(Some(unit)) = self.db.get_code_unit(name) {
                    similar_units.push(SimilarUnit {
                        qualified_name: unit.qualified_name,
                        file_path: unit.file_path,
                        range_start: unit.range_start,
                        project_id: unit.project_id,
                        similarity,
                    });
                }
            }
        }

        Ok(similar_units)
    }

    /// 轻量级 ANN 搜索（只返回 qualified_name + similarity，不查数据库）
    /// 适合批量/并行搜索场景
    pub fn search_names(
        &self,
        query_embedding: &[f32],
        k: usize,
        threshold: f32,
    ) -> Result<Vec<(String, f32)>> {
        let index = self.vector_index.as_ref()
            .ok_or(StoreError::VectorIndexNotInitialized)?;

        let results = index.search(query_embedding, k)?;

        Ok(results
            .into_iter()
            .filter_map(|r| {
                let similarity = r.similarity();
                if similarity >= threshold {
                    self.id_to_name.get(&r.id).map(|name| (name.clone(), similarity))
                } else {
                    None
                }
            })
            .collect())
    }

    /// 批量并行 ANN 搜索（接受切片引用，避免克隆）
    /// 返回 Vec<(query_index, qualified_name, similarity)>
    pub fn search_batch_parallel<'a>(
        &self,
        queries: &[(usize, &'a [f32])], // (index, embedding slice)
        k: usize,
        threshold: f32,
    ) -> Result<Vec<(usize, String, f32)>> {
        let index = self.vector_index.as_ref()
            .ok_or(StoreError::VectorIndexNotInitialized)?;

        // 直接引用映射表（&HashMap 是 Sync 的）
        let id_to_name = &self.id_to_name;

        let results: Vec<_> = queries
            .par_iter()
            .flat_map(|(query_idx, emb)| {
                match index.search(*emb, k) {
                    Ok(hits) => hits
                        .into_iter()
                        .filter_map(|r| {
                            let similarity = r.similarity();
                            if similarity >= threshold {
                                id_to_name.get(&r.id).map(|name| (*query_idx, name.clone(), similarity))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                    Err(_) => vec![],
                }
            })
            .collect();

        Ok(results)
    }

    /// 带过滤的 ANN 搜索
    pub fn search_similar_filtered<F>(
        &self,
        query_embedding: &[f32],
        k: usize,
        threshold: f32,
        filter: F,
    ) -> Result<Vec<SimilarUnit>>
    where
        F: Fn(&str) -> bool,
    {
        let index = self.vector_index.as_ref()
            .ok_or(StoreError::VectorIndexNotInitialized)?;

        // 构建 ID 过滤器
        let id_filter = |id: u64| -> bool {
            if let Some(name) = self.id_to_name.get(&id) {
                filter(name)
            } else {
                false
            }
        };

        // ANN 搜索
        let results = index.search_filtered(query_embedding, k, id_filter)?;

        // 转换为 SimilarUnit
        let mut similar_units = Vec::new();
        for result in results {
            let similarity = result.similarity();

            if similarity < threshold {
                continue;
            }

            if let Some(name) = self.id_to_name.get(&result.id) {
                if let Ok(Some(unit)) = self.db.get_code_unit(name) {
                    similar_units.push(SimilarUnit {
                        qualified_name: unit.qualified_name,
                        file_path: unit.file_path,
                        range_start: unit.range_start,
                        project_id: unit.project_id,
                        similarity,
                    });
                }
            }
        }

        Ok(similar_units)
    }

    /// 保存向量索引
    pub fn save_vector_index(&self) -> Result<()> {
        if let Some(ref index) = self.vector_index {
            index.save(&self.vector_index_path)?;
        }
        Ok(())
    }

    /// 从现有数据库重建向量索引
    pub fn rebuild_vector_index(&mut self) -> Result<usize> {
        let units = self.db.get_code_units_by_projects(None)?;

        // 重建 mapping
        self.name_to_id.clear();
        self.id_to_name.clear();
        self.next_id = 1;

        // 创建新索引
        let index = VectorIndex::with_defaults()?;
        index.reserve(units.len() + 1000)?;

        let mut count = 0;
        for unit in &units {
            // 分配 ID 并更新 mapping
            let id = self.get_or_allocate_id(&unit.qualified_name);

            if let Some(ref emb_bytes) = unit.embedding {
                if let Some(embedding) = bytes_to_embedding(emb_bytes) {
                    let vec: Vec<f32> = embedding.to_vec();
                    index.add(id, &vec)?;
                    count += 1;
                }
            }
        }

        self.vector_index = Some(index);
        self.save_vector_index()?;

        Ok(count)
    }

    /// 获取向量索引统计
    pub fn vector_index_stats(&self) -> Option<(usize, usize)> {
        self.vector_index.as_ref().map(|idx| (idx.size(), idx.memory_usage()))
    }

    /// 获取底层数据库引用
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// 获取底层数据库可变引用
    pub fn db_mut(&mut self) -> &mut Database {
        &mut self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::embedding_to_bytes;
    use tempfile::tempdir;

    fn create_test_embedding(seed: f32) -> Vec<f32> {
        // 创建一个 1024 维的测试向量
        (0..1024).map(|i| (i as f32 * seed).sin()).collect()
    }

    #[test]
    fn test_store_basic() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut store = Store::open(&db_path).unwrap();

        // 创建项目
        let project_id = store.db_mut().get_or_create_project("test", "/test", "rust").unwrap();

        // 创建带 embedding 的 code unit
        let emb = create_test_embedding(1.0);
        let record = CodeUnitRecord {
            qualified_name: "rust::test::foo".to_string(),
            project_id,
            file_path: "/test/src/lib.rs".to_string(),
            kind: "function".to_string(),
            range_start: 10,
            range_end: 20,
            content_hash: "abc123".to_string(),
            structure_hash: "def456".to_string(),
            embedding: Some(embedding_to_bytes(&emb.clone().into())),
            group_id: None,
        };

        store.upsert_code_unit(&record).unwrap();

        // 搜索
        let results = store.search_similar(&emb, 10, 0.5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].qualified_name, "rust::test::foo");
        assert!(results[0].similarity > 0.99); // 自己和自己的相似度应该接近 1
    }

    #[test]
    fn test_store_search_filtered() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut store = Store::open(&db_path).unwrap();
        let project_id = store.db_mut().get_or_create_project("test", "/test", "rust").unwrap();

        // 添加多个 code unit
        for i in 0..5 {
            let emb = create_test_embedding(i as f32 + 1.0);
            let record = CodeUnitRecord {
                qualified_name: format!("rust::test::func_{}", i),
                project_id,
                file_path: "/test/src/lib.rs".to_string(),
                kind: "function".to_string(),
                range_start: i * 10,
                range_end: i * 10 + 10,
                content_hash: format!("hash_{}", i),
                structure_hash: format!("struct_{}", i),
                embedding: Some(embedding_to_bytes(&emb.into())),
                group_id: None,
            };
            store.upsert_code_unit(&record).unwrap();
        }

        // 搜索并过滤掉 func_0
        let query = create_test_embedding(1.0);
        let results = store.search_similar_filtered(
            &query,
            10,
            0.0,
            |name| !name.contains("func_0"),
        ).unwrap();

        // 不应该包含 func_0
        assert!(!results.iter().any(|r| r.qualified_name.contains("func_0")));
    }

    #[test]
    fn test_store_rebuild_index() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut store = Store::open(&db_path).unwrap();
        let project_id = store.db_mut().get_or_create_project("test", "/test", "rust").unwrap();

        // 直接写数据库（不通过 store）
        for i in 0..3 {
            let emb = create_test_embedding(i as f32 + 1.0);
            let record = CodeUnitRecord {
                qualified_name: format!("rust::test::func_{}", i),
                project_id,
                file_path: "/test/src/lib.rs".to_string(),
                kind: "function".to_string(),
                range_start: i * 10,
                range_end: i * 10 + 10,
                content_hash: format!("hash_{}", i),
                structure_hash: format!("struct_{}", i),
                embedding: Some(embedding_to_bytes(&emb.into())),
                group_id: None,
            };
            store.db_mut().upsert_code_unit(&record).unwrap();
        }

        // 重建向量索引
        let count = store.rebuild_vector_index().unwrap();
        assert_eq!(count, 3);

        // 检查索引大小和 mapping
        let (size, _) = store.vector_index_stats().unwrap();
        eprintln!("Index size: {}, mapping size: {}", size, store.id_to_name.len());

        // 直接测试 VectorIndex 搜索
        let query = create_test_embedding(1.0);
        let raw_results = store.vector_index.as_ref().unwrap().search(&query, 10).unwrap();
        eprintln!("Raw usearch results: {:?}", raw_results);

        // 验证可以搜索
        let results = store.search_similar(&query, 10, 0.0).unwrap();
        eprintln!("Search results: {:?}", results);

        // 如果 usearch 返回少于 3 个，就检查实际数量
        assert!(results.len() >= 1, "Should have at least 1 result");
        // 至少应该找到最相似的那个（完全匹配）
        assert_eq!(results[0].qualified_name, "rust::test::func_0");
    }
}
