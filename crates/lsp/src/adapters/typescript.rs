use crate::protocol::{LspClient, Result, LspError};
use crate::types::{CodeUnit, CallHierarchy, CallHierarchyItem};
use super::LanguageAdapter;
use async_trait::async_trait;
use lsp_types::{DocumentSymbol, SymbolKind};
use std::fs;
use std::path::Path;

/// TypeScript/JavaScript 语言适配器 (typescript-language-server)
pub struct TypeScriptAdapter {
    workspace: String,
    client: LspClient,
    initialized: bool,
}

impl TypeScriptAdapter {
    pub fn new(workspace: &str) -> Self {
        Self {
            workspace: workspace.to_string(),
            client: LspClient::new(workspace),
            initialized: false,
        }
    }

    /// 查找 typescript-language-server 路径
    fn find_tsserver() -> Option<String> {
        // PATH 中查找
        if let Ok(output) = std::process::Command::new("which")
            .arg("typescript-language-server")
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }

        // 常见路径
        let common_paths = [
            "/usr/local/bin/typescript-language-server",
            "/opt/homebrew/bin/typescript-language-server",
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
                Some(p) => format!("ts:{}::{}::{}", file_path, p, symbol.name),
                None => format!("ts:{}::{}", file_path, symbol.name),
            };

            // Function = 12, Method = 6, Constructor = 9
            if matches!(symbol.kind, SymbolKind::FUNCTION | SymbolKind::METHOD | SymbolKind::CONSTRUCTOR) {
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
                        SymbolKind::METHOD => "method",
                        _ => "function",
                    }.to_string(),
                    range_start,
                    range_end,
                    body,
                    selection_line: symbol.selection_range.start.line,
                    selection_column: symbol.selection_range.start.character,
                });
            }

            // 递归处理子符号 (class 内的方法)
            if let Some(children) = &symbol.children {
                let new_parent = if matches!(symbol.kind, SymbolKind::CLASS | SymbolKind::INTERFACE | SymbolKind::OBJECT) {
                    Some(symbol.name.as_str())
                } else {
                    parent_name
                };
                self.extract_functions(children, file_path, content, new_parent, units);
            }
        }
    }

    /// 获取文件的语言标识符
    fn get_language_id(file_path: &str) -> &'static str {
        let path = Path::new(file_path);
        match path.extension().and_then(|e| e.to_str()) {
            Some("tsx") => "typescriptreact",
            Some("jsx") => "javascriptreact",
            Some("js") | Some("mjs") | Some("cjs") => "javascript",
            _ => "typescript",
        }
    }
}

#[async_trait]
impl LanguageAdapter for TypeScriptAdapter {
    async fn start(&mut self) -> Result<()> {
        let tsserver_path = Self::find_tsserver()
            .ok_or_else(|| LspError::Protocol("typescript-language-server not found. Install with: npm install -g typescript-language-server typescript".into()))?;

        self.client.start(&tsserver_path, &["--stdio"])?;

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        self.client.initialize().await?;
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
        collect_ts_files(Path::new(&self.workspace), &mut files)?;
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

/// 递归收集 TypeScript/JavaScript 文件
fn collect_ts_files(dir: &Path, files: &mut Vec<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // 跳过常见的非源码目录
    let skip_dirs = [
        "node_modules",
        "dist",
        "build",
        ".next",
        ".nuxt",
        "coverage",
        ".git",
        ".turbo",
        ".cache",
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
            collect_ts_files(&path, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            // 收集 .ts, .tsx, .js, .jsx, .mjs, .cjs
            if matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
                // 跳过声明文件和配置文件
                let file_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !file_name.ends_with(".d.ts")
                    && !file_name.ends_with(".config.ts")
                    && !file_name.ends_with(".config.js")
                    && !file_name.ends_with(".config.mjs")
                {
                    files.push(path.to_string_lossy().to_string());
                }
            }
        }
    }

    Ok(())
}
