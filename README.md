# Iris

LSP-powered code analysis toolkit for Rust and Swift.

## Installation

```bash
cargo install --path .
```

## Dependencies

- [rust-analyzer](https://rust-analyzer.github.io/) - Rust project analysis
- [sourcekit-lsp](https://github.com/apple/sourcekit-lsp) - Swift project analysis
- [Ollama](https://ollama.ai/) + bge-m3 model - vector embeddings

```bash
ollama pull bge-m3
```

## Usage

### akin - Code Similarity Detection

```bash
# Index project
iris akin index /path/to/project -l rust

# Scan for similar code
iris akin scan --all -t 0.85

# Cross-project comparison
iris akin compare /project-a --lang-a swift /project-b --lang-b swift

# View status
iris akin status /path/to/project
iris akin projects
iris akin pairs -s new -l 20

# Ignore pairs
iris akin ignore "module::func_a" "module::func_b"

# Group management
iris akin group create "utils" -r "common utilities"
iris akin group add 1 "module::helper"
iris akin group list
```

### arch - Architecture Analysis

```bash
# Generate call graph
iris arch diagram /path/to/project -l rust
iris arch diagram /path/to/project -l swift -m  # module level

# Detect dead code
iris arch dead-code /path/to/project -l rust
iris arch dead-code /path/to/project --json

# Call tree analysis
iris arch call-tree /path/to/project main -l rust -d 5
iris arch call-tree /path/to/project foo -i  # incoming: who calls it
```

### Claude Code Hook

```bash
cargo install --path crates/akin  # installs akin-hook
```

```bash
export AKIN_DB_PATH="$HOME/.akin/akin.db"
export AKIN_SIMILARITY_THRESHOLD=0.85
```

```json
// ~/.claude/settings.json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": { "tool_name": "edit|write" },
      "command": "akin-hook"
    }]
  }
}
```

## Library Usage

```rust
use lsp::RustAdapter;
use arch::ArchitectureAnalyzer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut adapter = RustAdapter::new("/path/to/project");
    adapter.start().await?;

    let mut analyzer = ArchitectureAnalyzer::new();
    analyzer.build_call_graph(&mut adapter).await?;

    for node in analyzer.find_dead_code() {
        println!("{}:{} {}", node.file_path, node.line, node.name);
    }
    Ok(())
}
```

## License

MIT
