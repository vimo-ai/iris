//! 数据库类型定义

use std::collections::HashMap;

/// 配对状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairStatus {
    New,
    Confirmed,
    Redundant,
    Ignored,
}

impl PairStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Confirmed => "confirmed",
            Self::Redundant => "redundant",
            Self::Ignored => "ignored",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "new" => Some(Self::New),
            "confirmed" => Some(Self::Confirmed),
            "redundant" => Some(Self::Redundant),
            "ignored" => Some(Self::Ignored),
            _ => None,
        }
    }
}

/// 项目记录
#[derive(Debug, Clone)]
pub struct ProjectRecord {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub language: String,
    pub last_indexed_at: Option<String>,
}

/// CodeUnit 数据库记录
#[derive(Debug, Clone)]
pub struct CodeUnitRecord {
    pub qualified_name: String,
    pub project_id: i64,
    pub file_path: String,
    pub kind: String,
    pub range_start: u32,
    pub range_end: u32,
    pub content_hash: String,
    pub structure_hash: String,
    pub embedding: Option<Vec<u8>>,
    pub group_id: Option<i64>,
}

/// 相似配对记录
#[derive(Debug, Clone)]
pub struct SimilarPairRecord {
    pub id: i64,
    pub unit_a: String,
    pub unit_b: String,
    pub similarity: f32,
    pub status: PairStatus,
    pub trigger_reason: Option<String>,
    // join 扩展字段
    pub file_a: Option<String>,
    pub start_a: Option<u32>,
    pub end_a: Option<u32>,
    pub file_b: Option<String>,
    pub start_b: Option<u32>,
    pub end_b: Option<u32>,
}

/// 相似度分组记录
#[derive(Debug, Clone)]
pub struct SimilarityGroupRecord {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub reason: Option<String>,
    pub pattern: Option<String>,
}

/// 项目统计信息
#[derive(Debug)]
pub struct ProjectStats {
    pub total_units: i64,
    pub pairs_by_status: HashMap<String, i64>,
    pub total_groups: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pair_status_conversion() {
        assert_eq!(PairStatus::New.as_str(), "new");
        assert_eq!(PairStatus::Confirmed.as_str(), "confirmed");
        assert_eq!(PairStatus::Redundant.as_str(), "redundant");
        assert_eq!(PairStatus::Ignored.as_str(), "ignored");

        assert_eq!(PairStatus::from_str("new"), Some(PairStatus::New));
        assert_eq!(PairStatus::from_str("confirmed"), Some(PairStatus::Confirmed));
        assert_eq!(PairStatus::from_str("invalid"), None);
    }
}
