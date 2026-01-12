//! arch - 架构分析
//!
//! 调用图分析、死码检测、文档生成

mod analyzer;
mod mermaid;

pub use analyzer::ArchitectureAnalyzer;
pub use mermaid::MermaidGenerator;
