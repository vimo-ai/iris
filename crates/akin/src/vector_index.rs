//! 向量索引模块 - 基于 usearch HNSW 算法的 ANN 搜索

use std::path::Path;
use thiserror::Error;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// 向量索引错误
#[derive(Error, Debug)]
pub enum VectorIndexError {
    #[error("usearch error: {0}")]
    Usearch(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
}

impl From<cxx::Exception> for VectorIndexError {
    fn from(e: cxx::Exception) -> Self {
        VectorIndexError::Usearch(e.what().to_string())
    }
}

pub type Result<T> = std::result::Result<T, VectorIndexError>;

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// 向量 ID (对应 code_unit 的 rowid)
    pub id: u64,
    /// 距离 (越小越相似，cosine distance = 1 - similarity)
    pub distance: f32,
}

impl SearchResult {
    /// 转换为相似度 (0-1，越大越相似)
    pub fn similarity(&self) -> f32 {
        1.0 - self.distance
    }
}

/// 向量索引配置
#[derive(Debug, Clone, Copy)]
pub struct VectorIndexConfig {
    /// 向量维度
    pub dimensions: usize,
    /// HNSW 连接数 (M)，越大越精确但越慢
    pub connectivity: usize,
    /// 扩展因子，影响构建质量
    pub expansion_add: usize,
    /// 搜索扩展因子，影响搜索精度
    pub expansion_search: usize,
}

impl Default for VectorIndexConfig {
    fn default() -> Self {
        Self {
            dimensions: 1024, // nomic-embed-text 维度
            connectivity: 16, // HNSW M 参数，0 表示自动选择
            expansion_add: 128, // 构建时的扩展因子
            expansion_search: 64, // 搜索时的扩展因子
        }
    }
}

impl VectorIndexConfig {
    /// 创建测试用的小维度配置
    pub fn for_test(dimensions: usize) -> Self {
        Self {
            dimensions,
            connectivity: 8,
            expansion_add: 64,
            expansion_search: 32,
        }
    }
}

/// 向量索引 - 封装 usearch HNSW 索引
pub struct VectorIndex {
    index: Index,
    config: VectorIndexConfig,
}

impl VectorIndex {
    /// 创建新的向量索引
    pub fn new(config: VectorIndexConfig) -> Result<Self> {
        let options = IndexOptions {
            dimensions: config.dimensions,
            metric: MetricKind::Cos, // 余弦距离
            quantization: ScalarKind::F32,
            connectivity: config.connectivity,
            expansion_add: config.expansion_add,
            expansion_search: config.expansion_search,
            multi: false, // 不允许重复 key
        };

        let index = Index::new(&options)?;
        Ok(Self { index, config })
    }

    /// 使用默认配置创建
    pub fn with_defaults() -> Result<Self> {
        Self::new(VectorIndexConfig::default())
    }

    /// 从文件加载索引
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with_config(path, VectorIndexConfig::default())
    }

    /// 使用指定配置从文件加载索引
    pub fn load_with_config(path: &Path, config: VectorIndexConfig) -> Result<Self> {
        let options = IndexOptions {
            dimensions: config.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: config.connectivity,
            expansion_add: config.expansion_add,
            expansion_search: config.expansion_search,
            multi: false,
        };

        let index = Index::new(&options)?;
        index.load(path.to_str().unwrap_or_default())?;

        Ok(Self { index, config })
    }

    /// 保存索引到文件
    pub fn save(&self, path: &Path) -> Result<()> {
        self.index.save(path.to_str().unwrap_or_default())?;
        Ok(())
    }

    /// 预分配容量
    pub fn reserve(&self, capacity: usize) -> Result<()> {
        self.index.reserve(capacity)?;
        Ok(())
    }

    /// 添加向量
    pub fn add(&self, id: u64, vector: &[f32]) -> Result<()> {
        if vector.len() != self.config.dimensions {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.config.dimensions,
                got: vector.len(),
            });
        }
        self.index.add(id, vector)?;
        Ok(())
    }

    /// 批量添加向量
    pub fn add_batch(&self, items: &[(u64, Vec<f32>)]) -> Result<()> {
        for (id, vector) in items {
            self.add(*id, vector)?;
        }
        Ok(())
    }

    /// 删除向量
    pub fn remove(&self, id: u64) -> Result<bool> {
        let count = self.index.remove(id)?;
        Ok(count > 0)
    }

    /// 检查是否包含向量
    pub fn contains(&self, id: u64) -> bool {
        self.index.contains(id)
    }

    /// 搜索最近邻
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        if query.len() != self.config.dimensions {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.config.dimensions,
                got: query.len(),
            });
        }

        let matches = self.index.search(query, k)?;
        let results: Vec<SearchResult> = matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .map(|(&id, &distance)| SearchResult { id, distance })
            .collect();

        Ok(results)
    }

    /// 带过滤的搜索
    pub fn search_filtered<F>(&self, query: &[f32], k: usize, filter: F) -> Result<Vec<SearchResult>>
    where
        F: Fn(u64) -> bool,
    {
        if query.len() != self.config.dimensions {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.config.dimensions,
                got: query.len(),
            });
        }

        let matches = self.index.filtered_search(query, k, &filter)?;
        let results: Vec<SearchResult> = matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .map(|(&id, &distance)| SearchResult { id, distance })
            .collect();

        Ok(results)
    }

    /// 获取索引大小
    pub fn size(&self) -> usize {
        self.index.size()
    }

    /// 获取索引容量
    pub fn capacity(&self) -> usize {
        self.index.capacity()
    }

    /// 获取内存使用量
    pub fn memory_usage(&self) -> usize {
        self.index.memory_usage()
    }

    /// 获取维度
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_index() {
        let index = VectorIndex::with_defaults().unwrap();
        assert_eq!(index.dimensions(), 1024);
        assert_eq!(index.size(), 0);
    }

    #[test]
    fn test_add_and_search() {
        let config = VectorIndexConfig::for_test(4);
        let index = VectorIndex::new(config).unwrap();

        // 先预分配容量
        index.reserve(10).unwrap();

        // 添加向量
        index.add(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.add(2, &[0.9, 0.1, 0.0, 0.0]).unwrap();
        index.add(3, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        assert_eq!(index.size(), 3);

        // 搜索
        let results = index.search(&[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, 1); // 最近的是自己
        assert_eq!(results[1].id, 2); // 其次是 id=2
    }

    #[test]
    fn test_dimension_mismatch() {
        let config = VectorIndexConfig::for_test(4);
        let index = VectorIndex::new(config).unwrap();

        let result = index.add(1, &[1.0, 0.0]); // 维度不匹配
        assert!(result.is_err());
    }

    #[test]
    fn test_search_filtered() {
        let config = VectorIndexConfig::for_test(4);
        let index = VectorIndex::new(config).unwrap();
        index.reserve(10).unwrap();

        index.add(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.add(2, &[0.9, 0.1, 0.0, 0.0]).unwrap();
        index.add(3, &[0.8, 0.2, 0.0, 0.0]).unwrap();

        // 过滤掉 id=1
        let results = index
            .search_filtered(&[1.0, 0.0, 0.0, 0.0], 2, |id| id != 1)
            .unwrap();

        assert!(!results.iter().any(|r| r.id == 1));
    }

    #[test]
    fn test_save_and_load() {
        let config = VectorIndexConfig::for_test(4);
        let index = VectorIndex::new(config.clone()).unwrap();
        index.reserve(10).unwrap();
        index.add(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.add(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        // 保存
        let temp_path = std::env::temp_dir().join("test_vector_index.usearch");
        index.save(&temp_path).unwrap();

        // 重新加载（使用相同配置）
        let loaded = VectorIndex::load_with_config(&temp_path, config).unwrap();
        assert_eq!(loaded.size(), 2);
        assert!(loaded.contains(1));
        assert!(loaded.contains(2));

        // 清理
        std::fs::remove_file(temp_path).ok();
    }

    #[test]
    fn test_similarity_conversion() {
        let result = SearchResult {
            id: 1,
            distance: 0.1, // cosine distance
        };
        assert!((result.similarity() - 0.9).abs() < 0.001);
    }
}
