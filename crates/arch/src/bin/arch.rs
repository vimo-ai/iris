//! arch CLI - 架构分析工具

use arch::{ArchitectureAnalyzer, MermaidGenerator, CallDirection};
use clap::{Parser, Subcommand};
use lsp::{LanguageAdapter, RustAdapter, SwiftAdapter};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "arch")]
#[command(about = "架构分析工具", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 生成 Mermaid 架构图
    Diagram {
        /// 项目路径
        path: String,
        /// 语言类型 (rust, swift)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// 生成模块级别图
        #[arg(short, long)]
        module: bool,
        /// 最大节点数
        #[arg(long, default_value = "100")]
        max_nodes: usize,
        /// 输出文件
        #[arg(short, long)]
        output: Option<String>,
    },
    /// 检测死代码
    DeadCode {
        /// 项目路径
        path: String,
        /// 语言类型 (rust, swift)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// 输出 JSON 格式
        #[arg(long)]
        json: bool,
    },
    /// 生成调用树
    CallTree {
        /// 项目路径
        path: String,
        /// 入口函数名
        entry: String,
        /// 语言类型 (rust, swift)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// 最大深度
        #[arg(short, long, default_value = "5")]
        depth: usize,
        /// 显示调用者 (默认显示被调用者)
        #[arg(short, long)]
        incoming: bool,
        /// 输出 JSON 格式
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Diagram { path, lang, module, max_nodes, output } => {
            cmd_diagram(&path, &lang, module, max_nodes, output.as_deref()).await?;
        }
        Commands::DeadCode { path, lang, json } => {
            cmd_dead_code(&path, &lang, json).await?;
        }
        Commands::CallTree { path, entry, lang, depth, incoming, json } => {
            cmd_call_tree(&path, &entry, &lang, depth, incoming, json).await?;
        }
    }

    Ok(())
}

// ==================== Diagram ====================

async fn cmd_diagram(path: &str, lang: &str, module: bool, max_nodes: usize, output: Option<&str>) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("分析: {}", project_path.display());

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("构建调用图...");
    match lang {
        "rust" => {
            let mut adapter = RustAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        "swift" => {
            let mut adapter = SwiftAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("不支持的语言: {}", lang),
    }

    let generator = MermaidGenerator::new().with_max_nodes(max_nodes);

    let mermaid = if module {
        println!("生成模块依赖图...");
        generator.generate_module_diagram(&analyzer, project_path.to_str().unwrap())
    } else {
        println!("生成调用图...");
        generator.generate_call_graph(&analyzer)
    };

    match output {
        Some(file) => {
            std::fs::write(file, format!("```mermaid\n{}\n```\n", mermaid))?;
            println!("已保存到: {}", file);
        }
        None => {
            println!("\n{}", mermaid);
        }
    }

    Ok(())
}

// ==================== Dead Code ====================

async fn cmd_dead_code(path: &str, lang: &str, json: bool) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("分析: {}", project_path.display());

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("构建调用图...");
    match lang {
        "rust" => {
            let mut adapter = RustAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        "swift" => {
            let mut adapter = SwiftAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("不支持的语言: {}", lang),
    }

    let dead_code = analyzer.find_dead_code();

    if json {
        #[derive(serde::Serialize)]
        struct DeadCodeItem {
            name: String,
            file: String,
            line: u32,
        }

        let items: Vec<_> = dead_code.iter().map(|node| DeadCodeItem {
            name: node.name.clone(),
            file: node.file_path.clone(),
            line: node.line,
        }).collect();

        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        println!("\n找到 {} 个潜在未引用的函数:\n", dead_code.len());
        for node in dead_code {
            let rel_path = node.file_path
                .strip_prefix(project_path.to_str().unwrap())
                .map(|s| s.trim_start_matches('/'))
                .unwrap_or(&node.file_path);
            println!("  {}:{}", rel_path, node.line);
            println!("    {}", short_name(&node.name));
            println!();
        }
    }

    Ok(())
}

// ==================== Call Tree ====================

async fn cmd_call_tree(path: &str, entry: &str, lang: &str, depth: usize, incoming: bool, json: bool) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("分析: {}", project_path.display());
    println!("入口: {}", entry);
    println!("方向: {}", if incoming { "调用者" } else { "被调用者" });

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("构建调用图...");
    match lang {
        "rust" => {
            let mut adapter = RustAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        "swift" => {
            let mut adapter = SwiftAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("不支持的语言: {}", lang),
    }

    let direction = if incoming { CallDirection::Incoming } else { CallDirection::Outgoing };
    let tree = analyzer.get_call_tree(entry, direction, depth);

    if tree.is_empty() {
        println!("\n未找到函数: {}", entry);
        return Ok(());
    }

    if json {
        #[derive(serde::Serialize)]
        struct TreeItem {
            name: String,
            depth: usize,
        }

        let items: Vec<_> = tree.iter().map(|n| TreeItem {
            name: n.name.clone(),
            depth: n.depth,
        }).collect();

        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        println!("\n调用树 ({}):\n", entry);
        for node in &tree {
            let indent = "  ".repeat(node.depth);
            println!("{}- {}", indent, short_name(&node.name));
        }
    }

    Ok(())
}

// ==================== Helpers ====================

fn short_name(name: &str) -> String {
    name.split("::").last().unwrap_or(name).to_string()
}
