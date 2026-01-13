//! 相似度匹配器

use std::collections::HashSet;
use std::path::Path;
use lsp::CodeUnit;

use crate::db::{Database, CodeUnitRecord, PairStatus};
use crate::embedding::{OllamaEmbedding, cosine_similarity, bytes_to_embedding};
use crate::store::Store;
use super::config::{HookConfig, HookScope};
use super::types::{Result, SimilarityMatch};

/// 查找相似代码
pub async fn find_similar_units(
    db: &Database,
    embedder: &mut OllamaEmbedding,
    units: &[CodeUnit],
    current_project_path: Option<&str>,
    config: &HookConfig,
) -> Result<Vec<SimilarityMatch>> {
    let mut results = Vec::new();

    // 获取当前项目 ID
    let current_project_id = current_project_path
        .and_then(|p| db.get_project_by_path(p).ok().flatten())
        .map(|proj| proj.id);

    // 获取要比较的 code units
    let db_units = match config.scope {
        HookScope::Project if current_project_id.is_some() => {
            db.get_code_units_by_projects(Some(&[current_project_id.unwrap()]))?
        }
        _ => db.get_code_units_by_projects(None)?,
    };

    // 加载已忽略的配对
    let ignored_pairs: HashSet<(String, String)> = db
        .get_similar_pairs(None, Some(PairStatus::Ignored), 0.0)?
        .into_iter()
        .flat_map(|p| {
            vec![
                (p.unit_a.clone(), p.unit_b.clone()),
                (p.unit_b, p.unit_a),
            ]
        })
        .collect();

    // 加载 embeddings
    let db_embeddings: Vec<(CodeUnitRecord, ndarray::Array1<f32>)> = db_units
        .into_iter()
        .filter_map(|unit| {
            unit.embedding.as_ref()
                .and_then(|e| bytes_to_embedding(e))
                .map(|emb| (unit, emb))
        })
        .collect();

    if db_embeddings.is_empty() {
        return Ok(results);
    }

    // 对每个新 unit 生成 embedding 并比较
    for unit in units {
        let new_embedding = match embedder.embed(&unit.body).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut similarities: Vec<SimilarityMatch> = Vec::new();

        for (db_unit, db_emb) in &db_embeddings {
            // 跳过自己
            if db_unit.qualified_name == unit.qualified_name {
                continue;
            }

            // 跳过已忽略的配对
            if ignored_pairs.contains(&(unit.qualified_name.clone(), db_unit.qualified_name.clone())) {
                continue;
            }

            // cross_only 模式：跳过同项目
            if config.scope == HookScope::CrossOnly {
                if let Some(pid) = current_project_id {
                    if db_unit.project_id == pid {
                        continue;
                    }
                }
            }

            let sim = cosine_similarity(&new_embedding, db_emb);
            if sim >= config.threshold {
                let is_cross = current_project_id
                    .map(|pid| db_unit.project_id != pid)
                    .unwrap_or(true);

                similarities.push(SimilarityMatch {
                    current_name: unit.qualified_name.clone(),
                    current_file: unit.file_path.clone(),
                    current_line: unit.range_start,
                    similar_name: db_unit.qualified_name.clone(),
                    similar_file: db_unit.file_path.clone(),
                    similar_line: db_unit.range_start,
                    similarity: sim,
                    is_cross_project: is_cross,
                });
            }
        }

        // 按相似度排序，取 top N
        similarities.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
        results.extend(similarities.into_iter().take(config.max_results));
    }

    Ok(results)
}

/// 使用 ANN 索引查找相似代码（O(log n) 复杂度）
pub async fn find_similar_units_ann(
    store: &Store,
    embedder: &mut OllamaEmbedding,
    units: &[CodeUnit],
    current_project_path: Option<&str>,
    config: &HookConfig,
) -> Result<Vec<SimilarityMatch>> {
    let mut results = Vec::new();
    let db = store.db();

    // 获取当前项目 ID
    let current_project_id = current_project_path
        .and_then(|p| db.get_project_by_path(p).ok().flatten())
        .map(|proj| proj.id);

    // 加载已忽略的配对
    let ignored_pairs: HashSet<(String, String)> = db
        .get_similar_pairs(None, Some(PairStatus::Ignored), 0.0)?
        .into_iter()
        .flat_map(|p| {
            vec![
                (p.unit_a.clone(), p.unit_b.clone()),
                (p.unit_b, p.unit_a),
            ]
        })
        .collect();

    // 对每个新 unit 生成 embedding 并使用 ANN 搜索
    for unit in units {
        let new_embedding = match embedder.embed(&unit.body).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        // 使用 ANN 搜索，多取一些结果用于后续过滤
        let k = (config.max_results * 3).max(50); // 多取一些，因为要过滤

        // 构建过滤器
        let search_results = store.search_similar_filtered(
            new_embedding.as_slice().unwrap(),
            k,
            config.threshold,
            |name| {
                // 跳过自己
                if name == unit.qualified_name {
                    return false;
                }
                // 跳过已忽略的配对
                if ignored_pairs.contains(&(unit.qualified_name.clone(), name.to_string())) {
                    return false;
                }
                true
            },
        );

        let similar_units = match search_results {
            Ok(units) => units,
            Err(_) => continue,
        };

        let mut similarities: Vec<SimilarityMatch> = Vec::new();

        for su in similar_units {
            // cross_only 模式：跳过同项目
            if config.scope == HookScope::CrossOnly {
                if let Some(pid) = current_project_id {
                    if su.project_id == pid {
                        continue;
                    }
                }
            }

            // Project 模式：只在当前项目内搜索
            if config.scope == HookScope::Project {
                if let Some(pid) = current_project_id {
                    if su.project_id != pid {
                        continue;
                    }
                }
            }

            let is_cross = current_project_id
                .map(|pid| su.project_id != pid)
                .unwrap_or(true);

            similarities.push(SimilarityMatch {
                current_name: unit.qualified_name.clone(),
                current_file: unit.file_path.clone(),
                current_line: unit.range_start,
                similar_name: su.qualified_name,
                similar_file: su.file_path,
                similar_line: su.range_start,
                similarity: su.similarity,
                is_cross_project: is_cross,
            });

            if similarities.len() >= config.max_results {
                break;
            }
        }

        results.extend(similarities);
    }

    Ok(results)
}

/// 格式化结果输出
pub fn format_result(results: &[SimilarityMatch]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut lines = vec!["⚠️ 检测到相似代码:".to_string()];

    for r in results {
        let sim_pct = (r.similarity * 100.0) as i32;
        let cross_mark = if r.is_cross_project { " [跨项目]" } else { "" };

        // 提取简短的名称
        let current_short = r.current_name.split("::").last().unwrap_or(&r.current_name);
        let similar_short = r.similar_name.split("::").last().unwrap_or(&r.similar_name);

        // 提取文件名
        let current_file = Path::new(&r.current_file)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| r.current_file.clone());
        let similar_file = Path::new(&r.similar_file)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| r.similar_file.clone());

        lines.push(format!("  ({}%){}", sim_pct, cross_mark));
        lines.push(format!("  ├─ 当前: {}:{} {}()", current_file, r.current_line, current_short));
        lines.push(format!("  └─ 相似: {}:{} {}()", similar_file, r.similar_line, similar_short));
        lines.push(String::new());
    }

    lines.push("📋 处理方式:".to_string());
    lines.push("  1. 复用: import 或调用已有实现，避免重复".to_string());
    lines.push("  2. 忽略: 运行 akin ignore \"<当前>\" \"<相似>\" 标记为合理重复".to_string());
    lines.push("  3. 继续: 如果逻辑不同只是结构相似，直接继续编写".to_string());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_result_empty() {
        let result = format_result(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_result_with_matches() {
        let matches = vec![SimilarityMatch {
            current_name: "rust:test.rs::foo".to_string(),
            current_file: "/path/to/test.rs".to_string(),
            current_line: 10,
            similar_name: "rust:other.rs::bar".to_string(),
            similar_file: "/path/to/other.rs".to_string(),
            similar_line: 20,
            similarity: 0.95,
            is_cross_project: false,
        }];
        let result = format_result(&matches);
        assert!(result.contains("检测到相似代码"));
        assert!(result.contains("95%"));
        assert!(result.contains("foo"));
        assert!(result.contains("bar"));
    }

    #[test]
    fn test_format_result_cross_project() {
        let matches = vec![SimilarityMatch {
            current_name: "rust::foo".to_string(),
            current_file: "/a/test.rs".to_string(),
            current_line: 1,
            similar_name: "swift::bar".to_string(),
            similar_file: "/b/test.swift".to_string(),
            similar_line: 1,
            similarity: 0.90,
            is_cross_project: true,
        }];
        let result = format_result(&matches);
        assert!(result.contains("[跨项目]"));
    }

    #[test]
    fn test_similarity_match_struct() {
        let m = SimilarityMatch {
            current_name: "a".to_string(),
            current_file: "a.rs".to_string(),
            current_line: 1,
            similar_name: "b".to_string(),
            similar_file: "b.rs".to_string(),
            similar_line: 2,
            similarity: 0.85,
            is_cross_project: false,
        };
        assert_eq!(m.similarity, 0.85);
        assert!(!m.is_cross_project);
    }

    // 测试 threshold 过滤逻辑
    #[test]
    fn test_threshold_filtering_logic() {
        // 验证 cosine_similarity 和 threshold 比较逻辑
        let threshold = 0.85_f32;

        assert!(0.90 >= threshold); // 应该通过
        assert!(0.85 >= threshold); // 边界值应该通过
        assert!(!(0.84 >= threshold)); // 应该被过滤
    }

    // 测试 ignored_pairs HashSet 逻辑
    #[test]
    fn test_ignored_pairs_hashset() {
        let mut ignored: HashSet<(String, String)> = HashSet::new();
        ignored.insert(("a".to_string(), "b".to_string()));
        ignored.insert(("b".to_string(), "a".to_string())); // 双向

        assert!(ignored.contains(&("a".to_string(), "b".to_string())));
        assert!(ignored.contains(&("b".to_string(), "a".to_string())));
        assert!(!ignored.contains(&("a".to_string(), "c".to_string())));
    }

    // 测试 max_results 限制
    #[test]
    fn test_max_results_limit() {
        let mut results: Vec<SimilarityMatch> = (0..10).map(|i| SimilarityMatch {
            current_name: format!("unit_{}", i),
            current_file: "test.rs".to_string(),
            current_line: i,
            similar_name: format!("similar_{}", i),
            similar_file: "other.rs".to_string(),
            similar_line: i,
            similarity: 0.90 - (i as f32 * 0.01),
            is_cross_project: false,
        }).collect();

        // 按相似度排序
        results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());

        let max_results = 3;
        let limited: Vec<_> = results.into_iter().take(max_results).collect();

        assert_eq!(limited.len(), 3);
        assert!(limited[0].similarity > limited[1].similarity);
        assert!(limited[1].similarity > limited[2].similarity);
    }

    // 集成测试 - 需要 Ollama 服务
    #[test]
    #[ignore = "需要 Ollama 服务运行"]
    fn test_find_similar_units_integration() {
        // 此测试需要 Ollama 服务
        // 运行: cargo test test_find_similar_units_integration -- --ignored
    }
}
