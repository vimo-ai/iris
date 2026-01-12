use crate::db::Database;
use crate::embedding::{cosine_similarity, OllamaEmbedding};
use lsp::{CodeUnit, LanguageAdapter};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("LSP error: {0}")]
    Lsp(String),
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Embedding error: {0}")]
    Embedding(#[from] crate::embedding::EmbeddingError),
}

pub type Result<T> = std::result::Result<T, ScanError>;

/// 相似对
#[derive(Debug, Clone)]
pub struct SimilarPair {
    pub unit_a: String,
    pub unit_b: String,
    pub similarity: f32,
}

/// 代码扫描器
pub struct Scanner {
    embedding: OllamaEmbedding,
    threshold: f32,
    min_lines: u32,
}

impl Scanner {
    pub fn new(model: &str) -> Self {
        Self {
            embedding: OllamaEmbedding::new(model),
            threshold: 0.85,
            min_lines: 3,
        }
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    pub fn with_min_lines(mut self, min_lines: u32) -> Self {
        self.min_lines = min_lines;
        self
    }

    /// 索引项目
    pub async fn index_project<A: LanguageAdapter>(
        &self,
        adapter: &mut A,
        _db: &Database,
    ) -> Result<Vec<CodeUnit>> {
        let units = adapter
            .get_functions()
            .await
            .map_err(|e| ScanError::Lsp(e.to_string()))?;

        // 过滤小函数
        let filtered: Vec<CodeUnit> = units
            .into_iter()
            .filter(|u| (u.range_end - u.range_start) >= self.min_lines)
            .collect();

        // TODO: 生成嵌入并存储到数据库

        Ok(filtered)
    }

    /// 扫描相似度
    pub async fn scan_similarities(
        &mut self,
        units: &[CodeUnit],
    ) -> Result<Vec<SimilarPair>> {
        let mut pairs = Vec::new();

        // 生成所有嵌入
        let mut embeddings = Vec::with_capacity(units.len());
        for unit in units {
            let emb = self.embedding.embed(&unit.body).await?;
            embeddings.push(emb);
        }

        // 两两比较
        for i in 0..units.len() {
            for j in (i + 1)..units.len() {
                let similarity = cosine_similarity(&embeddings[i], &embeddings[j]);
                if similarity >= self.threshold {
                    pairs.push(SimilarPair {
                        unit_a: units[i].qualified_name.clone(),
                        unit_b: units[j].qualified_name.clone(),
                        similarity,
                    });
                }
            }
        }

        // 按相似度降序排序 (NaN 视为最小值)
        pairs.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(pairs)
    }
}
