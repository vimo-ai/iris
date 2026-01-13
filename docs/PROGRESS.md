# Iris 迁移进度

> Python akin → Rust iris

## 总览

| Crate | 核心功能 | CLI | 测试 | 状态 |
|-------|----------|-----|------|------|
| lsp | ✅ | - | ✅ | 完成 |
| akin | ✅ | ✅ | ✅ | **完成** (含 ANN 优化) |
| arch | ✅ | ✅ | ✅ | **完成** |

## lsp crate ✅

| 文件 | Python 对应 | 状态 | 说明 |
|------|-------------|------|------|
| `types.rs` | `lsp/base.py` | ✅ | CodeUnit, CallHierarchyItem 等 |
| `protocol.rs` | `lsp/base.py` | ✅ | LSP JSON-RPC 协议 |
| `adapters/rust.rs` | `lsp/rust.py` | ✅ | rust-analyzer 适配 |
| `adapters/swift.rs` | `lsp/swift.py` | ✅ | sourcekit-lsp 适配 |

**单元测试**: 11 个 ✅

## akin crate ✅

| 文件 | Python 对应 | 状态 | 说明 |
|------|-------------|------|------|
| `embedding.rs` | `embedding.py` | ✅ | Ollama 嵌入 |
| `scanner.rs` | `scanner.py` | ✅ | 相似度扫描 |
| `db.rs` | `db.py` | ✅ | 完整 CRUD 操作 |
| `hook.rs` | `hook.py` | ✅ | Claude Code PostToolUse hook |
| `bin/akin.rs` | `cli.py` | ✅ | 完整 CLI (含 ANN) |
| `bin/hook.rs` | - | ✅ | hook 入口 |

**单元测试**: 26 个 ✅

### db.rs 已实现

- `get_or_create_project` - 项目管理
- `upsert_code_unit` - CodeUnit CRUD
- `upsert_similar_pair` - 相似配对 CRUD
- `get_similar_pairs` - 按状态/相似度查询
- `update_pair_status` - 更新配对状态
- `create_group` / `add_to_group` - 分组管理
- `get_stats` - 项目统计

### hook.rs 已实现

- `HookConfig` - 环境变量配置
- `HookResult` - JSON 输出格式
- `CodeParser` - tree-sitter 代码解析 (Rust/Swift)
- `find_similar_units` - 相似度查找
- `handle_post_tool_use` - PostToolUse 事件处理
- `run_hook` - 主入口

**依赖**: tree-sitter 0.22, tree-sitter-rust 0.21, tree-sitter-swift 0.5

### CLI 命令进度

| 命令 | 状态 | 说明 |
|------|------|------|
| `scan` | ✅ | ANN 并行搜索，292x 加速 |
| `compare` | ✅ | 跨项目比较 (ANN) |
| `index` | ✅ | 索引到数据库 + 向量索引 |
| `status` | ✅ | 项目状态 |
| `projects` | ✅ | 列出项目 |
| `pairs` | ✅ | 列出配对 |
| `ignore` | ✅ | 忽略配对 |
| `group *` | ✅ | 分组管理 |

### ANN 向量索引 (2026-01-13 新增)

| 文件 | 功能 | 说明 |
|------|------|------|
| `vector_index.rs` | usearch HNSW 封装 | O(log n) 近似最近邻 |
| `store.rs` | DB + VectorIndex 协调 | 自动构建索引 |

**性能提升**:
- scan: 240s → 0.82s (292x)
- hook: O(n) → O(log n) (204x)

## arch crate ✅

| 文件 | Python 对应 | 状态 | 说明 |
|------|-------------|------|------|
| `analyzer.rs` | `arch.py` | ✅ | 调用图、死代码检测 |
| `mermaid.rs` | `arch.py` | ✅ | Mermaid 图生成 |
| `bin/arch.rs` | `cli.py` | ✅ | CLI 入口 (diagram, dead-code, call-tree) |

**单元测试**: 20 个 ✅

### arch CLI 命令

| 命令 | 功能 | 选项 |
|------|------|------|
| `diagram <path>` | 生成 Mermaid 架构图 | `-m` 模块图, `--max-nodes` |
| `dead-code <path>` | 检测死代码 | `--json` JSON 输出 |
| `call-tree <path> <entry>` | 调用树分析 | `-i` incoming, `-d` 深度 |

## 下一步优先级

(无待办项)

## 验证记录

### 2026-01-13 (ANN 优化)

- ✅ usearch HNSW 向量索引集成
- ✅ Store 协调层 (DB + VectorIndex)
- ✅ Hook ANN 搜索: O(n) → O(log n), 204x 加速
- ✅ scan 命令: 240s → 0.82s, 292x 加速
  - 并行 ANN 搜索 (rayon)
  - 批量事务写入
- ✅ index 命令: 使用 Store, 同步更新向量索引
- ✅ 性能测试: 7848 code units, 18012 相似配对

### 2026-01-13

- ✅ 单元测试全部通过 (57 个: 26 akin + 20 arch + 11 lsp)
- ✅ db.rs CRUD 完整实现
- ✅ hook.rs Claude Code hook 完整实现
- ✅ tree-sitter Rust/Swift 解析验证通过

### 2024-01-13

- ✅ 单元测试全部通过 (41 个)
- ✅ Swift 跨项目分析验证
  - CoreNetworkKit (362 函数) vs Vlaude (170 函数)
  - 发现 75 对相似代码
  - 最高相似度 97.98%
