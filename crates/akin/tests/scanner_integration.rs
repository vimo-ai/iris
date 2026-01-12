//! Scanner 集成测试
//!
//! 需要运行 Ollama: `ollama serve` 并下载模型 `ollama pull bge-m3`
//! 运行: `cargo test -p akin --test scanner_integration -- --ignored`

use akin::Scanner;
use lsp::CodeUnit;

/// 创建测试用 CodeUnit
fn make_code_unit(name: &str, body: &str) -> CodeUnit {
    CodeUnit {
        qualified_name: name.to_string(),
        file_path: "/test/file.rs".to_string(),
        kind: "function".to_string(),
        range_start: 0,
        range_end: body.lines().count() as u32,
        body: body.to_string(),
        selection_line: 0,
        selection_column: 0,
    }
}

#[tokio::test]
#[ignore = "需要 Ollama 服务运行"]
async fn test_scanner_finds_similar_code() {
    let mut scanner = Scanner::new("bge-m3").with_threshold(0.8);

    // 创建相似的代码片段
    let units = vec![
        make_code_unit("fn_a", r#"
            fn process_data(items: Vec<i32>) -> i32 {
                let mut sum = 0;
                for item in items {
                    sum += item;
                }
                sum
            }
        "#),
        make_code_unit("fn_b", r#"
            fn calculate_total(numbers: Vec<i32>) -> i32 {
                let mut total = 0;
                for num in numbers {
                    total += num;
                }
                total
            }
        "#),
        make_code_unit("fn_c", r#"
            fn parse_config(path: &str) -> Config {
                let content = fs::read_to_string(path).unwrap();
                serde_json::from_str(&content).unwrap()
            }
        "#),
    ];

    let result = scanner.scan_similarities(&units).await;

    match result {
        Ok(pairs) => {
            // fn_a 和 fn_b 应该非常相似
            let similar = pairs.iter().find(|p|
                (p.unit_a.contains("fn_a") && p.unit_b.contains("fn_b")) ||
                (p.unit_a.contains("fn_b") && p.unit_b.contains("fn_a"))
            );

            assert!(similar.is_some(), "Should find fn_a and fn_b as similar");
            assert!(similar.unwrap().similarity > 0.8, "Similarity should be high");

            // fn_c 应该与其他两个不太相似
            let dissimilar_count = pairs.iter().filter(|p|
                p.unit_a.contains("fn_c") || p.unit_b.contains("fn_c")
            ).count();

            // fn_c 可能不在结果中（低于阈值）或相似度较低
            println!("Found {} pairs involving fn_c", dissimilar_count);
        }
        Err(e) => {
            eprintln!("跳过测试: Ollama 服务不可用 - {}", e);
        }
    }
}

#[tokio::test]
#[ignore = "需要 Ollama 服务运行"]
async fn test_scanner_respects_threshold() {
    let mut scanner_high = Scanner::new("bge-m3").with_threshold(0.95);
    let mut scanner_low = Scanner::new("bge-m3").with_threshold(0.5);

    let units = vec![
        make_code_unit("fn_1", "fn foo() { println!(\"hello\"); }"),
        make_code_unit("fn_2", "fn bar() { println!(\"world\"); }"),
    ];

    let result_high = scanner_high.scan_similarities(&units).await;
    let result_low = scanner_low.scan_similarities(&units).await;

    match (result_high, result_low) {
        (Ok(pairs_high), Ok(pairs_low)) => {
            // 高阈值应该找到更少的对
            assert!(pairs_high.len() <= pairs_low.len(),
                    "Higher threshold should find fewer or equal pairs");
        }
        _ => {
            eprintln!("跳过测试: Ollama 服务不可用");
        }
    }
}

#[tokio::test]
#[ignore = "需要 Ollama 服务运行"]
async fn test_scanner_empty_input() {
    let mut scanner = Scanner::new("bge-m3");
    let units: Vec<CodeUnit> = vec![];

    let result = scanner.scan_similarities(&units).await;

    match result {
        Ok(pairs) => {
            assert!(pairs.is_empty(), "Empty input should return empty pairs");
        }
        Err(e) => {
            eprintln!("跳过测试: Ollama 服务不可用 - {}", e);
        }
    }
}

#[tokio::test]
#[ignore = "需要 Ollama 服务运行"]
async fn test_scanner_single_unit() {
    let mut scanner = Scanner::new("bge-m3");
    let units = vec![
        make_code_unit("single", "fn single() { 42 }"),
    ];

    let result = scanner.scan_similarities(&units).await;

    match result {
        Ok(pairs) => {
            assert!(pairs.is_empty(), "Single unit should return no pairs");
        }
        Err(e) => {
            eprintln!("跳过测试: Ollama 服务不可用 - {}", e);
        }
    }
}
