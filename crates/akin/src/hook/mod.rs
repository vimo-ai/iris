//! Claude Code PostToolUse hook - 实时代码相似度检查

mod config;
mod types;
mod parser;
mod matcher;

pub use config::*;
pub use types::*;
pub use parser::*;
pub use matcher::{find_similar_units, find_similar_units_ann, format_result};

use crate::db::Database;
use crate::embedding::OllamaEmbedding;
use crate::store::Store;
use std::process::Command;

/// 检查并自动索引新项目
fn ensure_project_indexed(db: &Database, cwd: Option<&str>) {
    let cwd = match cwd {
        Some(c) => c,
        None => return,
    };

    // 检查项目是否已索引
    if let Ok(Some(_)) = db.get_project_by_path(cwd) {
        return; // 已索引
    }

    // 未索引，spawn 后台进程
    // 获取 akin 二进制路径（与 akin-hook 同目录）
    if let Ok(hook_path) = std::env::current_exe() {
        if let Some(bin_dir) = hook_path.parent() {
            let akin_path = bin_dir.join("akin");
            if akin_path.exists() {
                // 后台执行 akin index <cwd>
                let _ = Command::new(&akin_path)
                    .args(["index", cwd])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                // 不等待，继续处理
            }
        }
    }
}

/// 处理 PostToolUse 事件
pub async fn handle_post_tool_use(input: &HookInput, config: &HookConfig) -> Result<HookResult> {
    // 获取文件路径和内容
    let tool_input = match &input.tool_input {
        Some(t) => t,
        None => return Ok(HookResult::empty()),
    };

    let file_path = match &tool_input.file_path {
        Some(p) => p,
        None => return Ok(HookResult::empty()),
    };

    let content = match &tool_input.content {
        Some(c) => c,
        None => return Ok(HookResult::empty()),
    };

    // 只处理代码文件
    if !is_code_file(file_path) {
        return Ok(HookResult::empty());
    }

    // 提取代码单元
    let mut parser = CodeParser::new();
    let units = parser.extract_functions(content, file_path, config.min_lines);
    if units.is_empty() {
        return Ok(HookResult::empty());
    }

    // 确保数据库目录存在并打开 Store
    let db_path = get_db_path();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 打开 Store（包含数据库和向量索引）
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(_) => return Ok(HookResult::empty()),
    };

    // 检查并自动索引新项目
    ensure_project_indexed(store.db(), input.cwd.as_deref());

    // 初始化 embedder
    let mut embedder = OllamaEmbedding::new(&config.model);

    // 根据向量索引状态选择搜索方式
    let results = if store.vector_index_stats().is_some() {
        // 使用 ANN 搜索（O(log n)）
        find_similar_units_ann(
            &store,
            &mut embedder,
            &units,
            input.cwd.as_deref(),
            config,
        ).await?
    } else {
        // 回退到暴力搜索（O(n)）
        find_similar_units(
            store.db(),
            &mut embedder,
            &units,
            input.cwd.as_deref(),
            config,
        ).await?
    };

    if results.is_empty() {
        return Ok(HookResult::empty());
    }

    // 格式化输出
    let message = format_result(&results);

    match config.notify {
        NotifyMode::Block => Ok(HookResult::block(message)),
        NotifyMode::User => Ok(HookResult::notify(message)),
    }
}

/// Hook 主入口
pub async fn run_hook() -> Result<()> {
    use std::io::Read;

    // 读取 stdin
    let mut stdin_data = String::new();
    std::io::stdin().read_to_string(&mut stdin_data)?;

    // 解析输入
    let input: HookInput = if stdin_data.is_empty() {
        HookInput {
            hook_event_name: None,
            tool_name: None,
            tool_input: None,
            cwd: None,
        }
    } else {
        serde_json::from_str(&stdin_data)?
    };

    // 加载配置
    let config = HookConfig::from_env();

    // 处理事件
    let result = match input.hook_event_name.as_deref() {
        Some("PostToolUse") => handle_post_tool_use(&input, &config).await?,
        _ => HookResult::empty(),
    };

    // 输出结果
    println!("{}", serde_json::to_string(&result)?);

    Ok(())
}
