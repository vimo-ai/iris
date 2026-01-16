use crate::protocol::{LspClient, Result, LspError};
use crate::types::{CodeUnit, CallHierarchy, CallHierarchyItem};
use super::LanguageAdapter;
use async_trait::async_trait;
use lsp_types::{DocumentSymbol, SymbolKind};
use std::fs;
use std::path::Path;

/// Swift 语言适配器 (sourcekit-lsp)
pub struct SwiftAdapter {
    workspace: String,
    client: LspClient,
    initialized: bool,
    /// Xcode 项目的 call hierarchy 不可用，跳过调用
    is_xcode_project: bool,
}

impl SwiftAdapter {
    pub fn new(workspace: &str) -> Self {
        let workspace_path = Path::new(workspace);

        let is_xcode_project = Self::detect_xcode_project(workspace_path);

        Self {
            workspace: workspace.to_string(),
            client: LspClient::new(workspace),
            initialized: false,
            is_xcode_project,
        }
    }

    /// 检测是否是 Xcode 项目 (非 SwiftPM)
    fn detect_xcode_project(workspace_path: &Path) -> bool {
        // 有 Package.swift 就是 SwiftPM
        if workspace_path.join("Package.swift").exists() {
            return false;
        }

        // 检查是否有 .xcodeproj 或 .xcworkspace
        fs::read_dir(workspace_path)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    let path = e.path();
                    let ext = path.extension().and_then(|e| e.to_str());
                    matches!(ext, Some("xcodeproj") | Some("xcworkspace"))
                })
            })
            .unwrap_or(false)
    }

    /// 查找 sourcekit-lsp 路径
    fn find_sourcekit_lsp() -> Option<String> {
        // Xcode 内置路径
        let xcode_path = "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/sourcekit-lsp";
        if Path::new(xcode_path).exists() {
            return Some(xcode_path.to_string());
        }

        // PATH 中查找
        if let Ok(output) = std::process::Command::new("which")
            .arg("sourcekit-lsp")
            .output()
        {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }

        None
    }

    /// 检测工作空间类型并返回适当的参数
    fn detect_workspace_args(&self) -> Vec<String> {
        let workspace_path = Path::new(&self.workspace);
        let mut args = Vec::new();

        // SwiftPM 项目不需要额外参数
        if workspace_path.join("Package.swift").exists() {
            return args;
        }

        // 检查是否是 Xcode 项目
        let has_xcodeproj = fs::read_dir(workspace_path)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    e.path().extension().map(|ext| ext == "xcodeproj").unwrap_or(false)
                })
            })
            .unwrap_or(false);

        let has_xcworkspace = fs::read_dir(workspace_path)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    e.path().extension().map(|ext| ext == "xcworkspace").unwrap_or(false)
                })
            })
            .unwrap_or(false);

        if has_xcodeproj || has_xcworkspace {
            args.push("--default-workspace-type".to_string());
            args.push("buildServer".to_string());

            if let Some(build_path) = self.find_derived_data_path() {
                args.push("--scratch-path".to_string());
                args.push(build_path);
            }
        }

        args
    }

    /// 带重试的 prepare_call_hierarchy
    /// sourcekitd 崩溃后会禁用 semantic editor 10 秒，需要等待恢复
    async fn prepare_call_hierarchy_with_retry(&mut self, unit: &CodeUnit) -> Result<Vec<lsp_types::CallHierarchyItem>> {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_SECS: u64 = 12; // semantic editor 禁用 10 秒，多等 2 秒

        for attempt in 0..MAX_RETRIES {
            match self.client.prepare_call_hierarchy(
                &unit.file_path,
                unit.selection_line,
                unit.selection_column,
            ).await {
                Ok(items) => return Ok(items),
                Err(e) => {
                    let error_msg = format!("{:?}", e);
                    // 处理 sourcekitd 崩溃相关的错误
                    let is_recoverable = error_msg.contains("semantic editor is disabled")
                        || error_msg.contains("connection interrupted");

                    if is_recoverable && attempt < MAX_RETRIES - 1 {
                        tracing::warn!("sourcekitd error, waiting {}s before retry ({}/{}): {}",
                            RETRY_DELAY_SECS, attempt + 1, MAX_RETRIES, error_msg);
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(LspError::Protocol("max retries exceeded".into()))
    }

    /// 尝试找到 Xcode 的 DerivedData 路径
    fn find_derived_data_path(&self) -> Option<String> {
        // 项目内的 DerivedData
        let local_derived = Path::new(&self.workspace).join("DerivedData");
        if local_derived.exists() {
            return Some(local_derived.to_string_lossy().to_string());
        }

        // 用户目录下的 DerivedData
        if let Some(home) = std::env::var("HOME").ok() {
            let user_derived = Path::new(&home)
                .join("Library/Developer/Xcode/DerivedData");
            if user_derived.exists() {
                return Some(user_derived.to_string_lossy().to_string());
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
                Some(p) => format!("swift:{}::{}::{}", file_path, p, symbol.name),
                None => format!("swift:{}::{}", file_path, symbol.name),
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

                // 清理函数名 (移除参数签名)
                let clean_name = symbol.name.split('(').next().unwrap_or(&symbol.name);

                units.push(CodeUnit {
                    qualified_name: qualified_name.replace(&symbol.name, clean_name),
                    file_path: file_path.to_string(),
                    kind: match symbol.kind {
                        SymbolKind::CONSTRUCTOR => "init",
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

            // 递归处理子符号
            if let Some(children) = &symbol.children {
                let new_parent = if matches!(symbol.kind, SymbolKind::CLASS | SymbolKind::STRUCT | SymbolKind::ENUM) {
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
impl LanguageAdapter for SwiftAdapter {
    async fn start(&mut self) -> Result<()> {
        let sourcekit_path = Self::find_sourcekit_lsp()
            .ok_or_else(|| LspError::Protocol("sourcekit-lsp not found".into()))?;

        let args = self.detect_workspace_args();
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.client.start(&sourcekit_path, &args_ref)?;

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        self.client.initialize().await?;
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

        for file_path in &files {
            let content = fs::read_to_string(file_path)
                .map_err(|e| LspError::Io(e))?;

            self.client.open_file(file_path, &content, "swift")?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            match self.client.document_symbols(file_path).await {
                Ok(symbols) => {
                    self.extract_functions(&symbols, file_path, &content, None, &mut units);
                }
                Err(_) => continue,
            }
        }

        Ok(units)
    }

    fn get_source_files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        collect_swift_files(Path::new(&self.workspace), &mut files)?;
        Ok(files)
    }

    async fn get_call_hierarchy(&mut self, unit: &CodeUnit) -> Result<CallHierarchy> {
        // Xcode 项目的 call hierarchy 不可用 (sourcekit-lsp 限制)
        if self.is_xcode_project {
            return Ok(CallHierarchy { incoming: vec![], outgoing: vec![] });
        }

        let items = match self.prepare_call_hierarchy_with_retry(unit).await {
            Ok(items) => items,
            Err(_) => return Ok(CallHierarchy { incoming: vec![], outgoing: vec![] }),
        };

        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();

        if let Some(item) = items.first() {
            if let Ok(callers) = self.client.incoming_calls(item).await {
                for call in callers {
                    incoming.push(CallHierarchyItem {
                        name: call.from.name.clone(),
                        file_path: call.from.uri.path().to_string(),
                        line: call.from.selection_range.start.line,
                    });
                }
            }

            if let Ok(callees) = self.client.outgoing_calls(item).await {
                for call in callees {
                    outgoing.push(CallHierarchyItem {
                        name: call.to.name.clone(),
                        file_path: call.to.uri.path().to_string(),
                        line: call.to.selection_range.start.line,
                    });
                }
            }
        }

        Ok(CallHierarchy { incoming, outgoing })
    }

    fn stop(&mut self) -> Result<()> {
        self.client.shutdown()
    }
}

/// 递归收集 .swift 文件
fn collect_swift_files(dir: &Path, files: &mut Vec<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // 跳过构建目录和第三方依赖
    let skip_dirs = [".build", "build", "Build", "DerivedData", "Pods", "SourcePackages", "Checkouts"];
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
            collect_swift_files(&path, files)?;
        } else if path.extension().map(|e| e == "swift").unwrap_or(false) {
            files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}
