use crate::protocol::{LspClient, Result, LspError};
use crate::types::{CodeUnit, CallHierarchy, CallHierarchyItem};
use super::LanguageAdapter;
use async_trait::async_trait;
use lsp_types::{DocumentSymbol, SymbolKind};
use std::fs;
use std::path::Path;

/// Java 语言适配器 (Eclipse JDT Language Server)
pub struct JavaAdapter {
    workspace: String,
    client: LspClient,
    initialized: bool,
}

impl JavaAdapter {
    pub fn new(workspace: &str) -> Self {
        Self {
            workspace: workspace.to_string(),
            client: LspClient::new(workspace),
            initialized: false,
        }
    }

    /// 查找 jdtls 路径
    fn find_jdtls() -> Option<String> {
        // PATH 中查找
        if let Ok(output) = std::process::Command::new("which")
            .arg("jdtls")
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }

        // 常见路径 (Homebrew / 手动安装)
        let common_paths = [
            "/usr/local/bin/jdtls",
            "/opt/homebrew/bin/jdtls",
        ];
        for path in common_paths {
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }

        None
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
                Some(p) => format!("java:{}::{}::{}", file_path, p, symbol.name),
                None => format!("java:{}::{}", file_path, symbol.name),
            };

            // Method = 6, Constructor = 9
            if matches!(symbol.kind, SymbolKind::METHOD | SymbolKind::CONSTRUCTOR) {
                let range_start = symbol.range.start.line;
                let range_end = symbol.range.end.line;

                let lines: Vec<&str> = content.lines().collect();
                let body = lines
                    .get(range_start as usize..=range_end as usize)
                    .map(|l| l.join("\n"))
                    .unwrap_or_default();

                units.push(CodeUnit {
                    qualified_name,
                    file_path: file_path.to_string(),
                    kind: match symbol.kind {
                        SymbolKind::CONSTRUCTOR => "constructor",
                        _ => "method",
                    }.to_string(),
                    range_start,
                    range_end,
                    body,
                    selection_line: symbol.selection_range.start.line,
                    selection_column: symbol.selection_range.start.character,
                });
            }

            // 递归处理子符号 (class/interface 内的方法)
            if let Some(children) = &symbol.children {
                let new_parent = if matches!(symbol.kind, SymbolKind::CLASS | SymbolKind::INTERFACE | SymbolKind::ENUM) {
                    Some(symbol.name.as_str())
                } else {
                    parent_name
                };
                self.extract_functions(children, file_path, content, new_parent, units);
            }
        }
    }

    /// 获取文件的语言标识符
    fn get_language_id(_file_path: &str) -> &'static str {
        "java"
    }
}

#[async_trait]
impl LanguageAdapter for JavaAdapter {
    async fn start(&mut self) -> Result<()> {
        let jdtls_path = Self::find_jdtls()
            .ok_or_else(|| LspError::Protocol("jdtls not found. Install with: brew install jdtls".into()))?;

        self.client.start(&jdtls_path, &[])?;

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        self.client.initialize().await?;
        // jdtls 初始化较慢，需要更长等待时间
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

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

            let lang_id = Self::get_language_id(&file_path);
            self.client.open_file(&file_path, &content, lang_id)?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let symbols = self.client.document_symbols(&file_path).await?;
            self.extract_functions(&symbols, &file_path, &content, None, &mut units);
        }

        Ok(units)
    }

    fn get_source_files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        collect_java_files(Path::new(&self.workspace), &mut files)?;
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
            let callers = self.client.incoming_calls(item).await?;
            for call in callers {
                incoming.push(CallHierarchyItem {
                    name: call.from.name.clone(),
                    file_path: call.from.uri.path().to_string(),
                    line: call.from.selection_range.start.line,
                });
            }

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

/// 递归收集 Java 源文件
fn collect_java_files(dir: &Path, files: &mut Vec<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // 跳过构建产物和非源码目录
    let skip_dirs = [
        "build",
        ".gradle",
        "target",
        ".git",
        ".idea",
        ".settings",
        "bin",
        "out",
        ".mvn",
        "node_modules",
    ];
    if dir.file_name()
        .map(|n| skip_dirs.iter().any(|&s| n == s))
        .unwrap_or(false)
    {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(LspError::Io)? {
        let entry = entry.map_err(LspError::Io)?;
        let path = entry.path();

        if path.is_dir() {
            collect_java_files(&path, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "java" {
                files.push(path.to_string_lossy().to_string());
            }
        }
    }

    Ok(())
}
