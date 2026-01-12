# Iris

Code analysis toolkit powered by LSP.

## Crates

- **lsp** - LSP client core (rust-analyzer, sourcekit-lsp)
- **akin** - Code redundancy detection via vector embeddings
- **arch** - Architecture analysis, dead code detection, Mermaid diagrams

## Usage

```rust
use lsp::RustAdapter;
use arch::ArchitectureAnalyzer;

#[tokio::main]
async fn main() {
    let mut adapter = RustAdapter::new("/path/to/project");
    adapter.start().await.unwrap();

    let mut analyzer = ArchitectureAnalyzer::new();
    analyzer.build_call_graph(&mut adapter).await.unwrap();

    let dead_code = analyzer.find_dead_code();
    println!("Found {} potentially unused functions", dead_code.len());
}
```

## Requirements

- rust-analyzer (for Rust projects)
- sourcekit-lsp (for Swift projects)
- Ollama with bge-m3 model (for akin embeddings)

## License

MIT
