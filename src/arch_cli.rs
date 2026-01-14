//! arch subcommand - architecture analysis

use arch::{ArchitectureAnalyzer, MermaidGenerator, CallDirection};
use clap::Subcommand;
use lsp::{LanguageAdapter, RustAdapter, SwiftAdapter, TypeScriptAdapter};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ArchCommands {
    /// Generate Mermaid diagram
    Diagram {
        /// Project path
        path: String,
        /// Language (rust, swift, typescript/ts)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// Generate module-level diagram
        #[arg(short, long)]
        module: bool,
        /// Max nodes
        #[arg(long, default_value = "100")]
        max_nodes: usize,
        /// Output file
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Detect dead code
    DeadCode {
        /// Project path
        path: String,
        /// Language (rust, swift, typescript/ts)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    /// Generate call tree
    CallTree {
        /// Project path
        path: String,
        /// Entry function name
        entry: String,
        /// Language (rust, swift, typescript/ts)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// Max depth
        #[arg(short, long, default_value = "5")]
        depth: usize,
        /// Show callers (default: callees)
        #[arg(short, long)]
        incoming: bool,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(cmd: ArchCommands) -> anyhow::Result<()> {
    match cmd {
        ArchCommands::Diagram { path, lang, module, max_nodes, output } => {
            cmd_diagram(&path, &lang, module, max_nodes, output.as_deref()).await
        }
        ArchCommands::DeadCode { path, lang, json } => {
            cmd_dead_code(&path, &lang, json).await
        }
        ArchCommands::CallTree { path, entry, lang, depth, incoming, json } => {
            cmd_call_tree(&path, &entry, &lang, depth, incoming, json).await
        }
    }
}

async fn cmd_diagram(path: &str, lang: &str, module: bool, max_nodes: usize, output: Option<&str>) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("Analyzing: {}", project_path.display());

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("Building call graph...");
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
        "typescript" | "ts" => {
            let mut adapter = TypeScriptAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("Unsupported language: {}", lang),
    }

    let generator = MermaidGenerator::new().with_max_nodes(max_nodes);

    let mermaid = if module {
        println!("Generating module diagram...");
        generator.generate_module_diagram(&analyzer, project_path.to_str().unwrap())
    } else {
        println!("Generating call graph...");
        generator.generate_call_graph(&analyzer)
    };

    match output {
        Some(file) => {
            std::fs::write(file, format!("```mermaid\n{}\n```\n", mermaid))?;
            println!("Saved to: {}", file);
        }
        None => {
            println!("\n{}", mermaid);
        }
    }

    Ok(())
}

async fn cmd_dead_code(path: &str, lang: &str, json: bool) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("Analyzing: {}", project_path.display());

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("Building call graph...");
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
        "typescript" | "ts" => {
            let mut adapter = TypeScriptAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("Unsupported language: {}", lang),
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
        println!("\nFound {} potentially unreferenced functions:\n", dead_code.len());
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

async fn cmd_call_tree(path: &str, entry: &str, lang: &str, depth: usize, incoming: bool, json: bool) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    println!("Analyzing: {}", project_path.display());
    println!("Entry: {}", entry);
    println!("Direction: {}", if incoming { "callers" } else { "callees" });

    let mut analyzer = ArchitectureAnalyzer::new();

    println!("Building call graph...");
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
        "typescript" | "ts" => {
            let mut adapter = TypeScriptAdapter::new(project_path.to_str().unwrap());
            adapter.start().await?;
            analyzer.build_call_graph(&mut adapter).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            adapter.stop()?;
        }
        _ => anyhow::bail!("Unsupported language: {}", lang),
    }

    let direction = if incoming { CallDirection::Incoming } else { CallDirection::Outgoing };
    let tree = analyzer.get_call_tree(entry, direction, depth);

    if tree.is_empty() {
        println!("\nFunction not found: {}", entry);
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
        println!("\nCall tree ({}):\n", entry);
        for node in &tree {
            let indent = "  ".repeat(node.depth);
            println!("{}- {}", indent, short_name(&node.name));
        }
    }

    Ok(())
}

fn short_name(name: &str) -> String {
    name.split("::").last().unwrap_or(name).to_string()
}
