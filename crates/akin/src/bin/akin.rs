//! akin CLI - 跨项目代码相似度分析

use akin::Scanner;
use clap::{Parser, Subcommand};
use lsp::{LanguageAdapter, RustAdapter, SwiftAdapter, CodeUnit};

#[derive(Parser)]
#[command(name = "akin")]
#[command(about = "代码冗余检测工具")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 扫描单个项目
    Scan {
        /// 项目路径
        path: String,
        /// 语言类型 (rust, swift)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// 相似度阈值
        #[arg(short, long, default_value = "0.85")]
        threshold: f32,
    },
    /// 跨项目对比分析
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Scan { path, lang, threshold } => {
            scan_project(&path, &lang, threshold).await?;
        }
        Commands::Compare { path_a, lang_a, path_b, lang_b, threshold } => {
            compare_projects(&path_a, &lang_a, &path_b, &lang_b, threshold).await?;
        }
    }

    Ok(())
}

async fn scan_project(path: &str, lang: &str, threshold: f32) -> anyhow::Result<()> {
    println!("扫描项目: {} ({})", path, lang);

    let units = extract_functions(path, lang).await?;
    println!("提取到 {} 个函数", units.len());

    if units.is_empty() {
        println!("没有找到函数");
        return Ok(());
    }

    let mut scanner = Scanner::new("bge-m3").with_threshold(threshold);
    println!("正在计算相似度...");

    let pairs = scanner.scan_similarities(&units).await?;

    println!("\n找到 {} 对相似代码 (阈值: {})", pairs.len(), threshold);

    // 过滤掉 init 方法的重复
    let filtered: Vec<_> = pairs.iter()
        .filter(|p| !p.unit_a.ends_with("::init") || !p.unit_b.ends_with("::init"))
        .take(20)
        .collect();

    for pair in &filtered {
        println!("\n相似度: {:.2}%", pair.similarity * 100.0);
        println!("  A: {}", format_name(&pair.unit_a));
        println!("  B: {}", format_name(&pair.unit_b));
    }

    let init_count = pairs.iter().filter(|p| p.unit_a.ends_with("::init") && p.unit_b.ends_with("::init")).count();
    if init_count > 0 {
        println!("\n(跳过 {} 对 init 方法)", init_count);
    }

    if pairs.len() > filtered.len() + init_count {
        println!("... 还有 {} 对", pairs.len() - filtered.len() - init_count);
    }

    Ok(())
}

async fn compare_projects(
    path_a: &str, lang_a: &str,
    path_b: &str, lang_b: &str,
    threshold: f32
) -> anyhow::Result<()> {
    println!("跨项目对比分析:");
    println!("  A: {} ({})", path_a, lang_a);
    println!("  B: {} ({})", path_b, lang_b);

    // 提取两个项目的函数
    let units_a = extract_functions(path_a, lang_a).await?;
    println!("项目A: {} 个函数", units_a.len());

    let units_b = extract_functions(path_b, lang_b).await?;
    println!("项目B: {} 个函数", units_b.len());

    if units_a.is_empty() || units_b.is_empty() {
        println!("至少有一个项目没有找到函数");
        return Ok(());
    }

    // 合并并分析
    let mut all_units = units_a.clone();
    all_units.extend(units_b.clone());

    let mut scanner = Scanner::new("bge-m3").with_threshold(threshold);
    println!("\n正在计算跨项目相似度...");

    let pairs = scanner.scan_similarities(&all_units).await?;

    // 过滤出跨项目的相似对
    let cross_pairs: Vec<_> = pairs.iter().filter(|p| {
        let a_in_proj_a = units_a.iter().any(|u| u.qualified_name == p.unit_a);
        let b_in_proj_b = units_b.iter().any(|u| u.qualified_name == p.unit_b);
        let a_in_proj_b = units_b.iter().any(|u| u.qualified_name == p.unit_a);
        let b_in_proj_a = units_a.iter().any(|u| u.qualified_name == p.unit_b);

        (a_in_proj_a && b_in_proj_b) || (a_in_proj_b && b_in_proj_a)
    }).collect();

    println!("\n找到 {} 对跨项目相似代码 (阈值: {})", cross_pairs.len(), threshold);
    for pair in cross_pairs.iter().take(30) {
        println!("\n相似度: {:.2}%", pair.similarity * 100.0);
        println!("  A: {}", format_name(&pair.unit_a));
        println!("  B: {}", format_name(&pair.unit_b));
    }

    if cross_pairs.len() > 30 {
        println!("\n... 还有 {} 对", cross_pairs.len() - 30);
    }

    Ok(())
}

async fn extract_functions(path: &str, lang: &str) -> anyhow::Result<Vec<CodeUnit>> {
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

fn format_name(name: &str) -> String {
    // swift:/path/to/file.swift::ClassName::methodName -> ClassName::methodName (file.swift)
    let parts: Vec<&str> = name.splitn(2, "::").collect();
    if parts.len() == 2 {
        let file_part = parts[0];
        let func_part = parts[1];

        // 提取文件名
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
