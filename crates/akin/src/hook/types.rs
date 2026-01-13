//! Hook 类型定义

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HookError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Embedding error: {0}")]
    Embedding(#[from] crate::embedding::EmbeddingError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, HookError>;

/// Hook 返回结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HookResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

impl HookResult {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn block(reason: String) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason),
            system_message: None,
        }
    }

    pub fn notify(message: String) -> Self {
        Self {
            decision: None,
            reason: None,
            system_message: Some(message),
        }
    }
}

/// Claude Code hook 输入
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub hook_event_name: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<ToolInput>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToolInput {
    pub file_path: Option<String>,
    pub content: Option<String>,
}

/// 相似度匹配结果
#[derive(Debug)]
pub struct SimilarityMatch {
    pub current_name: String,
    pub current_file: String,
    pub current_line: u32,
    pub similar_name: String,
    pub similar_file: String,
    pub similar_line: u32,
    pub similarity: f32,
    pub is_cross_project: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_result_serialization() {
        let empty = HookResult::empty();
        let json = serde_json::to_string(&empty).unwrap();
        assert_eq!(json, "{}");

        let block = HookResult::block("test reason".to_string());
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"decision\":\"block\""));
        assert!(json.contains("\"reason\":\"test reason\""));

        let notify = HookResult::notify("test message".to_string());
        let json = serde_json::to_string(&notify).unwrap();
        assert!(json.contains("\"systemMessage\":\"test message\""));
    }
}
