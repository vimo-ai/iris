//! akin - 代码冗余检测
//!
//! 基于向量嵌入的代码相似度分析工具

mod db;
mod embedding;
mod scanner;

pub use db::Database;
pub use embedding::OllamaEmbedding;
pub use scanner::{Scanner, SimilarPair};
