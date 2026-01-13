# Iris 架构设计

> 从 Python akin 项目迁移到 Rust workspace

## 项目结构

```
iris/
├── Cargo.toml           # workspace 配置
├── crates/
│   ├── lsp/             # LSP 协议层
│   ├── akin/            # 代码相似度分析
│   └── arch/            # 架构分析
└── docs/
```

## Crate 职责

### lsp

LSP (Language Server Protocol) 协议封装和语言适配器。

```
lsp/
├── src/
│   ├── lib.rs
│   ├── types.rs         # CodeUnit, CallHierarchyItem 等核心类型
│   ├── protocol.rs      # LSP JSON-RPC 协议实现
│   └── adapters/
│       ├── mod.rs
│       ├── rust.rs      # rust-analyzer 适配器
│       └── swift.rs     # sourcekit-lsp 适配器
```

**依赖关系**: 无内部依赖，底层 crate

### akin

代码相似度分析核心，提供索引、扫描、Hook 功能。

```
akin/
├── src/
│   ├── lib.rs
│   ├── embedding.rs     # Ollama 向量嵌入
│   ├── scanner.rs       # 相似度扫描器
│   ├── db.rs            # SQLite 持久化
│   ├── hook.rs          # Claude Code hook (TODO)
│   └── bin/
│       └── akin.rs      # CLI 入口
```

**依赖关系**: `lsp`

### arch

代码架构分析，调用图、死代码检测、可视化。

```
arch/
├── src/
│   ├── lib.rs
│   ├── analyzer.rs      # 架构分析器 (调用图、死代码)
│   └── mermaid.rs       # Mermaid 图生成
```

**依赖关系**: `lsp`

## 数据流

```
                    ┌─────────────┐
                    │  LSP Server │
                    │ (外部进程)  │
                    └──────┬──────┘
                           │ JSON-RPC
                           ▼
┌──────────────────────────────────────────┐
│                   lsp                     │
│  ┌─────────┐  ┌──────────┐  ┌─────────┐  │
│  │ protocol│  │  types   │  │adapters │  │
│  └─────────┘  └──────────┘  └─────────┘  │
└──────────────────────────────────────────┘
          │                        │
          ▼                        ▼
┌─────────────────┐      ┌─────────────────┐
│      akin       │      │      arch       │
│  ┌───────────┐  │      │  ┌───────────┐  │
│  │ embedding │  │      │  │ analyzer  │  │
│  ├───────────┤  │      │  ├───────────┤  │
│  │  scanner  │  │      │  │  mermaid  │  │
│  ├───────────┤  │      │  └───────────┘  │
│  │    db     │  │      └─────────────────┘
│  ├───────────┤  │
│  │   hook    │  │
│  └───────────┘  │
└─────────────────┘
```

## 外部依赖

| 依赖 | 用途 |
|------|------|
| rust-analyzer | Rust LSP |
| sourcekit-lsp | Swift LSP |
| Ollama (bge-m3) | 向量嵌入 |
| SQLite | 持久化存储 |
| tree-sitter | Hook 代码解析 (TODO) |

## CLI 命令设计

### akin CLI

```bash
# 基础扫描
akin scan <path> --lang <rust|swift> --threshold 0.85

# 跨项目比较
akin compare <path_a> --lang-a <lang> <path_b> --lang-b <lang>

# 索引管理 (TODO)
akin index <path> --lang <lang>
akin status <path>
akin projects

# 配对管理 (TODO)
akin pairs --status <new|confirmed|ignored>
akin ignore <pair_id>

# 分组管理 (TODO)
akin group create <name>
akin group add <group_id> <unit_name>
akin group list
```

### arch CLI (TODO)

```bash
arch diagram <path> --lang <lang> --output mermaid
arch dead-code <path> --lang <lang>
arch call-tree <entry_fn> --depth 5 --direction outgoing
```

## Hook 系统设计

Claude Code PostToolUse hook，实时检测代码相似度。

```
┌─────────────┐     stdin      ┌─────────────┐
│ Claude Code │ ─────────────► │  akin hook  │
│  (caller)   │                │  (binary)   │
└─────────────┘                └──────┬──────┘
                                      │
                    ┌─────────────────┼─────────────────┐
                    ▼                 ▼                 ▼
             ┌───────────┐    ┌───────────┐    ┌───────────┐
             │tree-sitter│    │  scanner  │    │    db     │
             │  parse    │    │  compare  │    │  lookup   │
             └───────────┘    └───────────┘    └───────────┘
                    │                 │                 │
                    └─────────────────┴─────────────────┘
                                      │
                                      ▼
                               ┌─────────────┐
                               │   stdout    │
                               │ {"decision":│
                               │  "block"}   │
                               └─────────────┘
```

**配置项** (环境变量):
- `AKIN_THRESHOLD`: 相似度阈值 (默认 0.85)
- `AKIN_MIN_LINES`: 最小行数 (默认 3)
- `AKIN_SCOPE`: 检查范围 (project|workspace)
- `AKIN_MAX_RESULTS`: 最大返回数 (默认 5)
