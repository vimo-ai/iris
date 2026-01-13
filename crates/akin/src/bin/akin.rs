//! akin CLI - 跨项目代码相似度分析

use akin::{
    Database, PairStatus, CodeUnitRecord, Store,
    OllamaEmbedding, embedding_to_bytes, bytes_to_embedding,
};
use akin::hook::get_db_path;
use clap::{Parser, Subcommand};
use lsp::{LanguageAdapter, RustAdapter, SwiftAdapter, CodeUnit};
use sha2::{Sha256, Digest};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "akin")]
#[command(about = "代码冗余检测工具", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 索引项目到数据库
    Index {
        /// 项目路径
        path: String,
        /// 语言类型 (rust, swift)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// 嵌入模型
        #[arg(short, long, default_value = "bge-m3")]
        model: String,
        /// 最小函数行数
        #[arg(long, default_value = "3")]
        min_lines: u32,
    },
    /// 扫描相似代码
    Scan {
        /// 项目路径（留空扫描所有已索引项目）
        paths: Vec<String>,
        /// 扫描所有已索引项目
        #[arg(short, long)]
        all: bool,
        /// 仅显示跨项目相似
        #[arg(short = 'x', long)]
        cross_only: bool,
        /// 相似度阈值
        #[arg(short, long, default_value = "0.85")]
        threshold: f32,
    },
    /// 跨项目对比分析（使用 LSP 实时提取，不依赖数据库）
    Compare {
        /// 项目A路径
        path_a: String,
        /// 项目A语言
        #[arg(long, default_value = "rust")]
        lang_a: String,
        /// 项目B路径
        path_b: String,
        /// 项目B语言
        #[arg(long, default_value = "swift")]
        lang_b: String,
        /// 相似度阈值
        #[arg(short, long, default_value = "0.80")]
        threshold: f32,
    },
    /// 显示项目状态
    Status {
        /// 项目路径
        path: String,
    },
    /// 列出所有已索引项目
    Projects,
    /// 列出相似配对
    Pairs {
        /// 过滤状态 (new, ignored, confirmed, redundant)
        #[arg(short, long, default_value = "new")]
        status: String,
        /// 最大显示数量
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// 标记配对为已忽略
    Ignore {
        /// 代码单元 A
        unit_a: String,
        /// 代码单元 B
        unit_b: String,
        /// 忽略原因
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// 分组管理
    #[command(subcommand)]
    Group(GroupCommands),
}

#[derive(Subcommand)]
enum GroupCommands {
    /// 创建分组
    Create {
        /// 分组名称
        name: String,
        /// 分组原因
        #[arg(short, long)]
        reason: String,
        /// 匹配模式
        #[arg(short, long)]
        pattern: Option<String>,
        /// 项目路径
        #[arg(short = 'P', long)]
        project: Option<String>,
    },
    /// 添加成员到分组
    Add {
        /// 分组 ID
        group_id: i64,
        /// 代码单元名称
        qualified_names: Vec<String>,
    },
    /// 列出分组
    List {
        /// 项目路径
        #[arg(short = 'P', long)]
        project: Option<String>,
    },
    /// 列出分组成员
    Members {
        /// 分组 ID
        group_id: i64,
    },
}

fn ensure_db() -> anyhow::Result<Database> {
    let db_path = get_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Database::open(&db_path)?)
}

fn ensure_store() -> anyhow::Result<Store> {
    let db_path = get_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Store::open(&db_path)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, lang, model, min_lines } => {
            cmd_index(&path, &lang, &model, min_lines).await?;
        }
        Commands::Scan { paths, all, cross_only, threshold } => {
            cmd_scan(&paths, all, cross_only, threshold).await?;
        }
        Commands::Compare { path_a, lang_a, path_b, lang_b, threshold } => {
            cmd_compare(&path_a, &lang_a, &path_b, &lang_b, threshold).await?;
        }
        Commands::Status { path } => {
            cmd_status(&path)?;
        }
        Commands::Projects => {
            cmd_projects()?;
        }
        Commands::Pairs { status, limit } => {
            cmd_pairs(&status, limit)?;
        }
        Commands::Ignore { unit_a, unit_b, reason } => {
            cmd_ignore(&unit_a, &unit_b, reason.as_deref())?;
        }
        Commands::Group(sub) => match sub {
            GroupCommands::Create { name, reason, pattern, project } => {
                cmd_group_create(&name, &reason, pattern.as_deref(), project.as_deref())?;
            }
            GroupCommands::Add { group_id, qualified_names } => {
                cmd_group_add(group_id, &qualified_names)?;
            }
            GroupCommands::List { project } => {
                cmd_group_list(project.as_deref())?;
            }
            GroupCommands::Members { group_id } => {
                cmd_group_members(group_id)?;
            }
        },
    }

    Ok(())
}

// ==================== Index ====================

async fn cmd_index(path: &str, lang: &str, model: &str, min_lines: u32) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    let project_name = project_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("项目: {}", project_path.display());
    println!("语言: {}", lang);
    println!("模型: {}", model);
    println!();

    let mut store = ensure_store()?;
    let project_id = store.db_mut().get_or_create_project(&project_name, project_path.to_str().unwrap(), lang)?;

    // 提取函数
    println!("提取代码单元...");
    let units = extract_functions_lsp(project_path.to_str().unwrap(), lang).await?;
    println!("找到 {} 个函数", units.len());

    // 过滤小函数
    let units: Vec<_> = units.into_iter()
        .filter(|u| (u.range_end - u.range_start) >= min_lines)
        .collect();
    println!("过滤后: {} 个函数 (>= {} 行)", units.len(), min_lines);

    if units.is_empty() {
        println!("没有找到符合条件的函数");
        return Ok(());
    }

    // 生成 embedding
    println!("\n生成向量嵌入...");
    let mut embedder = OllamaEmbedding::new(model);
    let mut indexed = 0;

    for (i, unit) in units.iter().enumerate() {
        print!("\r  [{}/{}] {}", i + 1, units.len(), short_name(&unit.qualified_name));

        let content_hash = compute_hash(&unit.body);
        let structure_hash = compute_structure_hash(&unit.body);

        // 检查缓存
        let embedding = if let Ok(Some(cached)) = store.db().get_embedding_by_content_hash(&content_hash) {
            cached
        } else {
            match embedder.embed(&unit.body).await {
                Ok(emb) => embedding_to_bytes(&emb),
                Err(e) => {
                    eprintln!("\n警告: 无法生成 embedding: {}", e);
                    continue;
                }
            }
        };

        let record = CodeUnitRecord {
            qualified_name: unit.qualified_name.clone(),
            project_id,
            file_path: unit.file_path.clone(),
            kind: unit.kind.clone(),
            range_start: unit.range_start,
            range_end: unit.range_end,
            content_hash,
            structure_hash,
            embedding: Some(embedding),
            group_id: None,
        };

        // 使用 Store 写入，同时更新数据库和向量索引
        store.upsert_code_unit(&record)?;
        indexed += 1;
    }

    // 保存向量索引
    store.save_vector_index()?;

    println!("\n\n索引完成: {} 个代码单元", indexed);
    if let Some((size, mem)) = store.vector_index_stats() {
        println!("向量索引: {} 条, 内存 {} KB", size, mem / 1024);
    }
    store.db_mut().update_project_indexed_time(project_id)?;

    Ok(())
}

// ==================== Scan ====================

async fn cmd_scan(paths: &[String], all: bool, cross_only: bool, threshold: f32) -> anyhow::Result<()> {
    use std::collections::HashSet;
    use std::time::Instant;

    let t0 = Instant::now();
    let store = ensure_store()?;
    let db = store.db();

    // 检查向量索引是否可用
    let has_vector_index = store.vector_index_stats().is_some();
    if !has_vector_index {
        println!("警告: 向量索引未初始化，将使用暴力搜索（较慢）");
    }

    // 确定要扫描的项目
    let project_ids: Vec<i64> = if all || paths.is_empty() {
        let projects = db.get_all_projects()?;
        if projects.is_empty() {
            println!("还没有索引任何项目。运行 'akin index <path>' 先索引项目。");
            return Ok(());
        }
        println!("扫描 {} 个项目: {}", projects.len(),
            projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
        projects.iter().map(|p| p.id).collect()
    } else {
        let mut ids = Vec::new();
        for p in paths {
            let resolved = PathBuf::from(p).canonicalize()?;
            match db.get_project_by_path(resolved.to_str().unwrap())? {
                Some(proj) => ids.push(proj.id),
                None => {
                    println!("项目未索引: {}", resolved.display());
                    println!("运行 'akin index {}' 先索引项目。", resolved.display());
                    return Ok(());
                }
            }
        }
        ids
    };

    // 获取代码单元
    let units = db.get_code_units_by_projects(Some(&project_ids))?;
    println!("加载 {} 个代码单元", units.len());

    if units.len() < 2 {
        println!("代码单元数量不足，无法比较");
        return Ok(());
    }

    // 加载 embeddings
    let units_with_emb: Vec<_> = units.iter()
        .filter_map(|u| {
            u.embedding.as_ref()
                .and_then(|e| bytes_to_embedding(e))
                .map(|emb| (u, emb))
        })
        .collect();
    println!("有效嵌入: {} 个", units_with_emb.len());

    if units_with_emb.len() < 2 {
        println!("有效嵌入数量不足，无法比较");
        return Ok(());
    }

    // 构建 qualified_name -> project_id 映射（用于跨项目过滤）
    let name_to_project: std::collections::HashMap<String, i64> = units.iter()
        .map(|u| (u.qualified_name.clone(), u.project_id))
        .collect();

    // 构建并行搜索查询
    let queries: Vec<(usize, &[f32])> = units_with_emb.iter()
        .enumerate()
        .map(|(i, (_, emb))| (i, emb.as_slice().unwrap()))
        .collect();

    // 并行 ANN 搜索
    print!("搜索相似代码...");
    let k = 100;
    let search_results = store.search_batch_parallel(&queries, k, threshold)?;

    // 处理搜索结果，去重
    let mut new_pairs: Vec<(String, String, f32)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for (query_idx, similar_name, similarity) in search_results {
        let query_name = &units_with_emb[query_idx].0.qualified_name;
        let query_project = units_with_emb[query_idx].0.project_id;

        // 跳过自己
        if &similar_name == query_name {
            continue;
        }

        // 跨项目过滤
        if cross_only {
            if let Some(&similar_project) = name_to_project.get(&similar_name) {
                if similar_project == query_project {
                    continue;
                }
            }
        }

        // 规范化配对顺序，避免重复
        let pair = if query_name < &similar_name {
            (query_name.clone(), similar_name.clone())
        } else {
            (similar_name.clone(), query_name.clone())
        };

        if seen.insert(pair.clone()) {
            new_pairs.push((pair.0, pair.1, similarity));
        }
    }

    // 批量写入数据库
    db.batch_upsert_similar_pairs(&new_pairs, Some("scan"))?;

    println!("\r完成: {} 对相似代码 (耗时 {:.2}s)", new_pairs.len(), t0.elapsed().as_secs_f32());

    // 显示结果
    let pairs = db.get_similar_pairs(None, None, threshold)?;

    // 过滤跨项目
    let pairs: Vec<_> = if cross_only && project_ids.len() > 1 {
        pairs.into_iter().filter(|p| {
            let a_proj = units.iter().find(|u| u.qualified_name == p.unit_a).map(|u| u.project_id);
            let b_proj = units.iter().find(|u| u.qualified_name == p.unit_b).map(|u| u.project_id);
            a_proj != b_proj
        }).collect()
    } else {
        pairs
    };

    println!("\n找到 {} 对相似代码 (阈值: {:.0}%)", pairs.len(), threshold * 100.0);
    println!("{}", "=".repeat(60));

    for (i, pair) in pairs.iter().take(20).enumerate() {
        let file_a = pair.file_a.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
        let file_b = pair.file_b.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
        let line_a = pair.start_a.unwrap_or(0);
        let line_b = pair.start_b.unwrap_or(0);

        println!("\n[{}] 相似度: {:.2}%", i + 1, pair.similarity * 100.0);
        println!("  A: {}:{} {}", file_a, line_a, short_name(&pair.unit_a));
        println!("  B: {}:{} {}", file_b, line_b, short_name(&pair.unit_b));
    }

    if pairs.len() > 20 {
        println!("\n... 还有 {} 对", pairs.len() - 20);
    }

    Ok(())
}

// ==================== Compare (LSP mode + ANN) ====================

async fn cmd_compare(
    path_a: &str, lang_a: &str,
    path_b: &str, lang_b: &str,
    threshold: f32
) -> anyhow::Result<()> {
    use akin::{VectorIndex, VectorIndexConfig};
    use std::collections::HashSet;
    use std::time::Instant;

    let t0 = Instant::now();

    println!("跨项目对比分析 (ANN):");
    println!("  A: {} ({})", path_a, lang_a);
    println!("  B: {} ({})", path_b, lang_b);

    let units_a = extract_functions_lsp(path_a, lang_a).await?;
    println!("项目A: {} 个函数", units_a.len());

    let units_b = extract_functions_lsp(path_b, lang_b).await?;
    println!("项目B: {} 个函数", units_b.len());

    if units_a.is_empty() || units_b.is_empty() {
        println!("至少有一个项目没有找到函数");
        return Ok(());
    }

    // 生成 embeddings
    println!("\n生成向量嵌入...");
    let mut embedder = OllamaEmbedding::new("bge-m3");

    let mut all_embeddings: Vec<(usize, String, Vec<f32>, bool)> = Vec::new(); // (idx, name, emb, is_project_a)

    // 项目 A 的 embeddings
    for (i, unit) in units_a.iter().enumerate() {
        print!("\r  A: [{}/{}]", i + 1, units_a.len());
        match embedder.embed(&unit.body).await {
            Ok(emb) => {
                let vec: Vec<f32> = emb.as_slice().unwrap().to_vec();
                all_embeddings.push((all_embeddings.len(), unit.qualified_name.clone(), vec, true));
            }
            Err(e) => eprintln!("\n警告: {}: {}", unit.qualified_name, e),
        }
    }
    println!();

    // 项目 B 的 embeddings
    for (i, unit) in units_b.iter().enumerate() {
        print!("\r  B: [{}/{}]", i + 1, units_b.len());
        match embedder.embed(&unit.body).await {
            Ok(emb) => {
                let vec: Vec<f32> = emb.as_slice().unwrap().to_vec();
                all_embeddings.push((all_embeddings.len(), unit.qualified_name.clone(), vec, false));
            }
            Err(e) => eprintln!("\n警告: {}: {}", unit.qualified_name, e),
        }
    }
    println!();

    if all_embeddings.len() < 2 {
        println!("有效嵌入数量不足");
        return Ok(());
    }

    // 构建临时向量索引
    println!("构建 ANN 索引...");
    let dimensions = all_embeddings[0].2.len();
    let config = VectorIndexConfig {
        dimensions,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
    };
    let index = VectorIndex::new(config)?;
    index.reserve(all_embeddings.len())?;

    for (idx, _, emb, _) in &all_embeddings {
        index.add(*idx as u64, emb)?;
    }

    // ANN 搜索跨项目相似
    println!("搜索跨项目相似代码...");
    let k = 50; // 每个查询返回的候选数
    let mut cross_pairs: Vec<(String, String, f32)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    // 项目 A 中的每个函数搜索项目 B 中的相似函数
    let project_a_indices: HashSet<u64> = all_embeddings.iter()
        .filter(|(_, _, _, is_a)| *is_a)
        .map(|(idx, _, _, _)| *idx as u64)
        .collect();

    for (_idx, name_a, emb, is_a) in &all_embeddings {
        if !*is_a {
            continue; // 只从项目 A 查询
        }

        // 搜索时过滤掉项目 A 的函数
        let results = index.search_filtered(emb, k, |id| !project_a_indices.contains(&id))?;

        for result in results {
            let similarity = result.similarity();
            if similarity < threshold {
                continue;
            }

            let name_b = &all_embeddings[result.id as usize].1;

            // 规范化顺序避免重复
            let pair = if name_a < name_b {
                (name_a.clone(), name_b.clone())
            } else {
                (name_b.clone(), name_a.clone())
            };

            if seen.insert(pair.clone()) {
                cross_pairs.push((pair.0, pair.1, similarity));
            }
        }
    }

    // 按相似度降序排序
    cross_pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    println!("\n找到 {} 对跨项目相似代码 (阈值: {:.0}%, 耗时 {:.2}s)",
        cross_pairs.len(), threshold * 100.0, t0.elapsed().as_secs_f32());

    for (i, (unit_a, unit_b, similarity)) in cross_pairs.iter().take(30).enumerate() {
        println!("\n[{}] 相似度: {:.2}%", i + 1, similarity * 100.0);
        println!("  A: {}", format_name(unit_a));
        println!("  B: {}", format_name(unit_b));
    }

    if cross_pairs.len() > 30 {
        println!("\n... 还有 {} 对", cross_pairs.len() - 30);
    }

    Ok(())
}

// ==================== Status ====================

fn cmd_status(path: &str) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    let db = ensure_db()?;

    match db.get_project_by_path(project_path.to_str().unwrap())? {
        Some(project) => {
            let stats = db.get_stats(project.id)?;

            println!("项目: {}", project.name);
            println!("路径: {}", project.root_path);
            println!("语言: {}", project.language);
            println!("上次索引: {}", project.last_indexed_at.unwrap_or_else(|| "从未".to_string()));
            println!();
            println!("代码单元: {}", stats.total_units);
            println!("相似分组: {}", stats.total_groups);
            println!();
            println!("相似配对统计:");
            for (status, count) in &stats.pairs_by_status {
                println!("  {}: {}", status, count);
            }
        }
        None => {
            println!("项目未索引: {}", project_path.display());
        }
    }

    Ok(())
}

// ==================== Projects ====================

fn cmd_projects() -> anyhow::Result<()> {
    let db = ensure_db()?;
    let projects = db.get_all_projects()?;

    if projects.is_empty() {
        println!("还没有索引任何项目。");
        return Ok(());
    }

    for project in projects {
        println!("[{}] {}", project.id, project.name);
        println!("    路径: {}", project.root_path);
        println!("    语言: {}", project.language);
        println!("    上次索引: {}", project.last_indexed_at.unwrap_or_else(|| "从未".to_string()));
        println!();
    }

    Ok(())
}

// ==================== Pairs ====================

fn cmd_pairs(status: &str, limit: usize) -> anyhow::Result<()> {
    let db = ensure_db()?;

    let pair_status = PairStatus::from_str(status)
        .ok_or_else(|| anyhow::anyhow!("无效状态: {}", status))?;

    let pairs = db.get_similar_pairs(None, Some(pair_status), 0.0)?;

    println!("相似配对 (状态: {}):", status);
    println!();

    for pair in pairs.iter().take(limit) {
        let file_a = pair.file_a.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
        let file_b = pair.file_b.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();

        println!("[{}] {:.2}%", pair.id, pair.similarity * 100.0);
        println!("  A: {}:{} {}", file_a, pair.start_a.unwrap_or(0), short_name(&pair.unit_a));
        println!("  B: {}:{} {}", file_b, pair.start_b.unwrap_or(0), short_name(&pair.unit_b));
        println!();
    }

    if pairs.len() > limit {
        println!("... 还有 {} 对", pairs.len() - limit);
    }

    Ok(())
}

// ==================== Ignore ====================

fn cmd_ignore(unit_a: &str, unit_b: &str, _reason: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;

    // 规范化顺序
    let (a, b) = if unit_a < unit_b { (unit_a, unit_b) } else { (unit_b, unit_a) };

    let pairs = db.get_similar_pairs(None, None, 0.0)?;
    let pair = pairs.iter().find(|p| p.unit_a == a && p.unit_b == b);

    match pair {
        Some(p) => {
            db.update_pair_status(p.id, PairStatus::Ignored)?;
            println!("已忽略配对 (相似度: {:.2}%):", p.similarity * 100.0);
            println!("  A: {}", a);
            println!("  B: {}", b);
        }
        None => {
            println!("配对未找到。");
            println!("该配对可能还未被检测为相似。");
        }
    }

    Ok(())
}

// ==================== Group ====================

fn cmd_group_create(name: &str, reason: &str, pattern: Option<&str>, project: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;

    let project_path = match project {
        Some(p) => PathBuf::from(p).canonicalize()?,
        None => std::env::current_dir()?,
    };

    let proj = db.get_project_by_path(project_path.to_str().unwrap())?
        .ok_or_else(|| anyhow::anyhow!("项目未索引: {}", project_path.display()))?;

    let group_id = db.create_group(proj.id, name, Some(reason), pattern)?;

    println!("创建分组 [{}] '{}'", group_id, name);
    println!("  原因: {}", reason);
    if let Some(p) = pattern {
        println!("  模式: {}", p);
    }
    println!();
    println!("添加成员: akin group add {} <qualified_name>", group_id);

    Ok(())
}

fn cmd_group_add(group_id: i64, qualified_names: &[String]) -> anyhow::Result<()> {
    let db = ensure_db()?;

    for qn in qualified_names {
        match db.get_code_unit(qn)? {
            Some(_) => {
                db.add_to_group(qn, group_id)?;
                println!("已添加到分组 {}: {}", group_id, qn);
            }
            None => {
                println!("警告: 代码单元未找到: {}", qn);
            }
        }
    }

    Ok(())
}

fn cmd_group_list(project: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;

    let groups = if let Some(p) = project {
        let project_path = PathBuf::from(p).canonicalize()?;
        let proj = db.get_project_by_path(project_path.to_str().unwrap())?
            .ok_or_else(|| anyhow::anyhow!("项目未索引: {}", project_path.display()))?;
        println!("项目 {} 的分组:", proj.name);
        db.get_groups(proj.id)?
    } else {
        println!("所有分组:");
        let mut all_groups = Vec::new();
        for proj in db.get_all_projects()? {
            all_groups.extend(db.get_groups(proj.id)?);
        }
        all_groups
    };

    if groups.is_empty() {
        println!("  (无)");
        return Ok(());
    }

    for g in groups {
        println!("\n[{}] {}", g.id, g.name);
        if let Some(reason) = &g.reason {
            println!("    原因: {}", reason);
        }
        if let Some(pattern) = &g.pattern {
            println!("    模式: {}", pattern);
        }
    }

    Ok(())
}

fn cmd_group_members(group_id: i64) -> anyhow::Result<()> {
    let db = ensure_db()?;

    let all_units = db.get_code_units_by_projects(None)?;
    let members: Vec<_> = all_units.iter()
        .filter(|u| u.group_id == Some(group_id))
        .collect();

    if members.is_empty() {
        println!("分组 {} 没有成员", group_id);
        return Ok(());
    }

    println!("分组 {} 的成员:", group_id);
    for unit in members {
        let file_name = Path::new(&unit.file_path).file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("  {}:{} {}", file_name, unit.range_start, short_name(&unit.qualified_name));
    }

    Ok(())
}

// ==================== Helpers ====================

async fn extract_functions_lsp(path: &str, lang: &str) -> anyhow::Result<Vec<CodeUnit>> {
    match lang {
        "rust" => {
            let mut adapter = RustAdapter::new(path);
            adapter.start().await?;
            let units = adapter.get_functions().await?;
            adapter.stop()?;
            Ok(units)
        }
        "swift" => {
            let mut adapter = SwiftAdapter::new(path);
            adapter.start().await?;
            let units = adapter.get_functions().await?;
            adapter.stop()?;
            Ok(units)
        }
        _ => {
            anyhow::bail!("不支持的语言: {}", lang);
        }
    }
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn compute_structure_hash(content: &str) -> String {
    // 简化的结构哈希：移除变量名和字面量
    let normalized = content.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    compute_hash(&normalized)
}

fn short_name(name: &str) -> String {
    name.split("::").last().unwrap_or(name).to_string()
}

fn format_name(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(2, "::").collect();
    if parts.len() == 2 {
        let file_part = parts[0];
        let func_part = parts[1];
        let file_name = file_part
            .rsplit('/')
            .next()
            .unwrap_or(file_part)
            .trim_start_matches("swift:")
            .trim_start_matches("rust:");
        format!("{} ({})", func_part, file_name)
    } else {
        name.to_string()
    }
}
