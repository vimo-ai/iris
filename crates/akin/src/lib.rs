//! akin - 代码冗余检测
//!
//! 基于向量嵌入的代码相似度分析工具

mod db;
mod embedding;
pub mod hook;
mod scanner;
mod store;
mod vector_index;

pub use db::{
    Database, PairStatus, ProjectRecord, CodeUnitRecord,
    SimilarPairRecord, SimilarityGroupRecord, ProjectStats
};
pub use embedding::{OllamaEmbedding, bytes_to_embedding, embedding_to_bytes, cosine_similarity};
pub use hook::{HookConfig, HookResult, HookInput, CodeParser, run_hook};
pub use scanner::{Scanner, SimilarPair};
pub use store::{Store, SimilarUnit, StoreError};
pub use vector_index::{VectorIndex, VectorIndexConfig, SearchResult, VectorIndexError};
