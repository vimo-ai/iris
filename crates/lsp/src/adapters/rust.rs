use crate::protocol::{LspClient, Result, LspError};
use crate::types::{CodeUnit, CallHierarchy, CallHierarchyItem};
use super::LanguageAdapter;
use async_trait::async_trait;
use lsp_types::{DocumentSymbol, SymbolKind};
use std::fs;
use std::path::Path;

/// Rust 语言适配器 (rust-analyzer)
pub struct RustAdapter {
    workspace: String,
    client: LspClient,
    initialized: bool,
}

impl RustAdapter {
    pub fn new(workspace: &str) -> Self {
        Self {
            workspace: workspace.to_string(),
            client: LspClient::new(workspace),
            initialized: false,
        }
    }

    /// 递归提取函数符号
    fn extract_functions(
        &self,
        symbols: &[DocumentSymbol],
        file_path: &str,
        content: &str,
        parent_name: Option<&str>,
        units: &mut Vec<CodeUnit>,
    ) {
        for symbol in symbols {
            let qualified_name = match parent_name {
                Some(p) => format!("rust:{}::{}::{}", file_path, p, symbol.name),
                None => format!("rust:{}::{}", file_path, symbol.name),
            };

            // Function = 12, Method = 6
            if matches!(symbol.kind, SymbolKind::FUNCTION | SymbolKind::METHOD) {
                let range_start = symbol.range.start.line;
                let range_end = symbol.range.end.line;

                // 提取函数体
                let lines: Vec<&str> = content.lines().collect();
                let body = lines
                    .get(range_start as usize..=range_end as usize)
                    .map(|l| l.join("\n"))
                    .unwrap_or_default();

                units.push(CodeUnit {
                    qualified_name,
                    file_path: file_path.to_string(),
                    kind: if symbol.kind == SymbolKind::METHOD { "method" } else { "function" }.to_string(),
                    range_start,
                    range_end,
                    body,
                    selection_line: symbol.selection_range.start.line,
                    selection_column: symbol.selection_range.start.character,
                });
            }

            // 递归处理子符号 (impl 块内的方法)
            if let Some(children) = &symbol.children {
                let new_parent = if matches!(symbol.kind, SymbolKind::CLASS | SymbolKind::STRUCT | SymbolKind::ENUM | SymbolKind::MODULE) {
                    Some(symbol.name.as_str())
                } else {
                    parent_name
                };
                self.extract_functions(children, file_path, content, new_parent, units);
            }
        }
    }
}

#[async_trait]
impl LanguageAdapter for RustAdapter {
    async fn start(&mut self) -> Result<()> {
        self.client.start("rust-analyzer", &[])?;

        // 等待初始化
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        self.client.initialize().await?;

        // 等待索引
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        self.initialized = true;
        Ok(())
    }

    async fn get_functions(&mut self) -> Result<Vec<CodeUnit>> {
        if !self.initialized {
            return Err(LspError::NotStarted);
        }

        let mut units = Vec::new();
        let files = self.get_source_files()?;

        for file_path in files {
            let content = fs::read_to_string(&file_path)
                .map_err(|e| LspError::Io(e))?;

            self.client.open_file(&file_path, &content, "rust")?;

            // 等待文件处理
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let symbols = self.client.document_symbols(&file_path).await?;
            self.extract_functions(&symbols, &file_path, &content, None, &mut units);
        }

        Ok(units)
    }

    fn get_source_files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        collect_rust_files(Path::new(&self.workspace), &mut files)?;
        Ok(files)
    }

    async fn get_call_hierarchy(&mut self, unit: &CodeUnit) -> Result<CallHierarchy> {
        let items = self.client.prepare_call_hierarchy(
            &unit.file_path,
            unit.selection_line,
            unit.selection_column,
        ).await?;

        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();

        if let Some(item) = items.first() {
            // 获取调用者
            let callers = self.client.incoming_calls(item).await?;
            for call in callers {
                incoming.push(CallHierarchyItem {
                    name: call.from.name.clone(),
                    file_path: call.from.uri.path().to_string(),
                    line: call.from.selection_range.start.line,
                });
            }

            // 获取被调用者
            let callees = self.client.outgoing_calls(item).await?;
            for call in callees {
                outgoing.push(CallHierarchyItem {
                    name: call.to.name.clone(),
                    file_path: call.to.uri.path().to_string(),
                    line: call.to.selection_range.start.line,
                });
            }
        }

        Ok(CallHierarchy { incoming, outgoing })
    }

    fn stop(&mut self) -> Result<()> {
        self.client.shutdown()
    }
}

/// 递归收集 .rs 文件
fn collect_rust_files(dir: &Path, files: &mut Vec<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // 跳过 target 目录
    if dir.file_name().map(|n| n == "target").unwrap_or(false) {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(LspError::Io)? {
        let entry = entry.map_err(LspError::Io)?;
        let path = entry.path();

        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}
