//! LSP 集成测试
//!
//! 需要安装 rust-analyzer: `rustup component add rust-analyzer`
//! 运行: `cargo test -p arch --test lsp_integration -- --ignored`

use lsp::{LanguageAdapter, RustAdapter};
use arch::{ArchitectureAnalyzer, MermaidGenerator};
use std::fs;
use tempfile::tempdir;

/// 创建临时 Rust 项目用于测试
fn create_test_project() -> tempfile::TempDir {
    let dir = tempdir().expect("Failed to create temp dir");

    // Cargo.toml
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "test_project"
version = "0.1.0"
edition = "2021"
"#
    ).expect("Failed to write Cargo.toml");

    // src/lib.rs
    fs::create_dir_all(dir.path().join("src")).expect("Failed to create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        r#"
pub fn main_entry() {
    let result = helper();
    println!("{}", result);
}

fn helper() -> i32 {
    inner_helper() + 1
}

fn inner_helper() -> i32 {
    42
}

pub fn unused_function() {
    // This function is never called
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper() {
        assert_eq!(helper(), 43);
    }
}
"#
    ).expect("Failed to write lib.rs");

    dir
}

#[tokio::test]
#[ignore = "需要 rust-analyzer"]
async fn test_rust_adapter_get_functions() {
    let project = create_test_project();
    let workspace = project.path().to_str().unwrap();

    let mut adapter = RustAdapter::new(workspace);

    // 启动 LSP
    if adapter.start().await.is_err() {
        eprintln!("跳过测试: rust-analyzer 未安装或启动失败");
        return;
    }

    // 获取函数
    let functions = adapter.get_functions().await.expect("Failed to get functions");

    // 验证找到了预期的函数
    let names: Vec<_> = functions.iter().map(|f| f.qualified_name.as_str()).collect();

    assert!(names.iter().any(|n| n.contains("main_entry")), "Should find main_entry");
    assert!(names.iter().any(|n| n.contains("helper")), "Should find helper");
    assert!(names.iter().any(|n| n.contains("inner_helper")), "Should find inner_helper");
    assert!(names.iter().any(|n| n.contains("unused_function")), "Should find unused_function");

    // 验证 CodeUnit 内容
    let main_fn = functions.iter().find(|f| f.qualified_name.contains("main_entry")).unwrap();
    assert_eq!(main_fn.kind, "function");
    assert!(main_fn.body.contains("helper()"));

    adapter.stop().expect("Failed to stop adapter");
}

#[tokio::test]
#[ignore = "需要 rust-analyzer"]
async fn test_call_hierarchy() {
    let project = create_test_project();
    let workspace = project.path().to_str().unwrap();

    let mut adapter = RustAdapter::new(workspace);

    if adapter.start().await.is_err() {
        eprintln!("跳过测试: rust-analyzer 未安装或启动失败");
        return;
    }

    let functions = adapter.get_functions().await.expect("Failed to get functions");
    let helper_fn = functions.iter().find(|f| f.qualified_name.contains("helper") && !f.qualified_name.contains("inner")).unwrap();

    // 获取调用层次
    let hierarchy = adapter.get_call_hierarchy(helper_fn).await.expect("Failed to get call hierarchy");

    // helper 被 main_entry 调用
    assert!(!hierarchy.incoming.is_empty() || !hierarchy.outgoing.is_empty(),
            "Helper should have call relationships");

    adapter.stop().expect("Failed to stop adapter");
}

#[tokio::test]
#[ignore = "需要 rust-analyzer"]
async fn test_full_architecture_analysis() {
    let project = create_test_project();
    let workspace = project.path().to_str().unwrap();

    let mut adapter = RustAdapter::new(workspace);

    if adapter.start().await.is_err() {
        eprintln!("跳过测试: rust-analyzer 未安装或启动失败");
        return;
    }

    // 构建调用图
    let mut analyzer = ArchitectureAnalyzer::new();
    analyzer.build_call_graph(&mut adapter).await.expect("Failed to build call graph");

    // 检测死代码
    let dead_code = analyzer.find_dead_code();
    let dead_names: Vec<_> = dead_code.iter().map(|n| n.name.as_str()).collect();

    // unused_function 应该被检测为死代码
    assert!(dead_names.iter().any(|n| n.contains("unused_function")),
            "Should detect unused_function as dead code");

    // 生成 Mermaid 图
    let generator = MermaidGenerator::new();
    let diagram = generator.generate_call_graph(&analyzer);

    assert!(diagram.starts_with("flowchart TD"), "Should generate valid Mermaid");

    adapter.stop().expect("Failed to stop adapter");
}

#[tokio::test]
#[ignore = "需要 rust-analyzer"]
async fn test_mermaid_module_diagram() {
    let project = create_test_project();
    let workspace = project.path().to_str().unwrap();

    let mut adapter = RustAdapter::new(workspace);

    if adapter.start().await.is_err() {
        eprintln!("跳过测试: rust-analyzer 未安装或启动失败");
        return;
    }

    let mut analyzer = ArchitectureAnalyzer::new();
    analyzer.build_call_graph(&mut adapter).await.expect("Failed to build call graph");

    let generator = MermaidGenerator::new();
    let module_diagram = generator.generate_module_diagram(&analyzer, workspace);

    assert!(module_diagram.starts_with("flowchart TD"), "Should generate valid Mermaid module diagram");

    adapter.stop().expect("Failed to stop adapter");
}
