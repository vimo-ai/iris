use lsp_types::*;
use serde::Deserialize;
use url::Url;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Error, Debug)]
pub enum LspError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("LSP error: {0}")]
    Protocol(String),
    #[error("Timeout")]
    Timeout,
    #[error("Process not started")]
    NotStarted,
}

pub type Result<T> = std::result::Result<T, LspError>;

/// LSP 客户端 - 管理与语言服务器的通信
pub struct LspClient {
    process: Option<Child>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    request_id: Arc<Mutex<i64>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    workspace: String,
}

impl LspClient {
    pub fn new(workspace: &str) -> Self {
        Self {
            process: None,
            stdin: None,
            request_id: Arc::new(Mutex::new(0)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            workspace: workspace.to_string(),
        }
    }

    /// 启动 LSP 服务器
    pub fn start(&mut self, command: &str, args: &[&str]) -> Result<()> {
        tracing::info!("Starting LSP: {} {:?} in {}", command, args, self.workspace);

        let mut child = Command::new(command)
            .args(args)
            .current_dir(&self.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().ok_or(LspError::NotStarted)?;
        let stdout = child.stdout.take().ok_or(LspError::NotStarted)?;
        let stderr = child.stderr.take();

        // stdin 需要在响应线程和主线程之间共享
        let stdin = Arc::new(Mutex::new(stdin));
        self.stdin = Some(Arc::clone(&stdin));

        // 启动 stderr 读取线程
        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(line) => tracing::warn!("[LSP stderr] {}", line),
                        Err(_) => break,
                    }
                }
            });
        }

        // 启动响应读取线程
        let pending = Arc::clone(&self.pending);
        std::thread::spawn(move || {
            Self::read_responses(stdout, pending, stdin);
        });

        self.process = Some(child);
        Ok(())
    }

    /// 向 stdin 写入 LSP 消息
    fn write_message(stdin: &Arc<Mutex<ChildStdin>>, msg: &str) -> std::result::Result<(), std::io::Error> {
        let header = format!("Content-Length: {}\r\n\r\n", msg.len());
        let mut stdin = stdin.lock().unwrap();
        stdin.write_all(header.as_bytes())?;
        stdin.write_all(msg.as_bytes())?;
        stdin.flush()
    }

    /// 读取 LSP 响应，并自动回复服务端→客户端请求
    fn read_responses(
        stdout: ChildStdout,
        pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
        stdin: Arc<Mutex<ChildStdin>>,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut buffer = String::new();

        loop {
            buffer.clear();
            let mut content_length: usize = 0;

            // 读取所有 headers 直到空行
            loop {
                buffer.clear();
                match reader.read_line(&mut buffer) {
                    Ok(0) => return, // EOF
                    Ok(_) => {}
                    Err(_) => return,
                }

                let line = buffer.trim();
                if line.is_empty() {
                    break; // headers 结束
                }

                if let Some(value) = line.strip_prefix("Content-Length:") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
                // 忽略其他 headers (如 Content-Type)
            }

            if content_length == 0 {
                continue;
            }

            // 读取 body
            let mut body = vec![0u8; content_length];
            if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                break;
            }

            if let Ok(msg) = serde_json::from_slice::<Value>(&body) {
                let has_id = msg.get("id").is_some();
                let has_method = msg.get("method").is_some();

                if has_id && has_method {
                    // 服务端→客户端请求 (同时有 id 和 method)
                    // 自动回复 null result，防止服务端阻塞
                    if let Some(id) = msg.get("id") {
                        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("unknown");
                        tracing::debug!("[LSP] 自动回复服务端请求: {} (id={})", method, id);
                        let response = json!({
                            "jsonrpc": "2.0",
                            "id": id.clone(),
                            "result": null
                        });
                        if let Ok(resp_str) = serde_json::to_string(&response) {
                            let _ = Self::write_message(&stdin, &resp_str);
                        }
                    }
                } else if has_id {
                    // 服务端响应 (只有 id，没有 method)
                    if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
                        if let Some(sender) = pending.lock().unwrap().remove(&id) {
                            let _ = sender.send(msg);
                        }
                    }
                } else if has_method {
                    // 纯通知 — 处理需要回复的特殊通知
                    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    if method == "tsserver/request" {
                        // vue-language-server (Volar) 专用: 通过通知转发 tsserver 请求
                        // 需要回复 tsserver/response 通知，否则 Volar 会阻塞
                        if let Some(params) = msg.get("params").and_then(|p| p.as_array()) {
                            for req in params {
                                if let Some(req_arr) = req.as_array() {
                                    if let Some(req_id) = req_arr.first() {
                                        tracing::debug!("[LSP] 回复 tsserver/request (id={})", req_id);
                                        let response = json!({
                                            "jsonrpc": "2.0",
                                            "method": "tsserver/response",
                                            "params": [[req_id.clone(), null]]
                                        });
                                        if let Ok(resp_str) = serde_json::to_string(&response) {
                                            let _ = Self::write_message(&stdin, &resp_str);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 发送请求
    pub async fn request<R: for<'de> Deserialize<'de>>(&mut self, method: &str, params: Value) -> Result<R> {
        let id = {
            let mut id = self.request_id.lock().unwrap();
            *id += 1;
            *id
        };

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let msg = serde_json::to_string(&request)?;

        // 先注册等待响应的 channel，避免竞态条件
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let stdin = self.stdin.as_ref().ok_or(LspError::NotStarted)?;
        Self::write_message(stdin, &msg).map_err(LspError::Io)?;

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            rx
        ).await
            .map_err(|_| LspError::Timeout)?
            .map_err(|_| LspError::Protocol("Channel closed".into()))?;

        if let Some(result) = response.get("result") {
            Ok(serde_json::from_value(result.clone())?)
        } else if let Some(error) = response.get("error") {
            Err(LspError::Protocol(error.to_string()))
        } else {
            Err(LspError::Protocol("No result".into()))
        }
    }

    /// 发送通知 (无响应)
    pub fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let msg = serde_json::to_string(&notification)?;
        let stdin = self.stdin.as_ref().ok_or(LspError::NotStarted)?;
        Self::write_message(stdin, &msg).map_err(LspError::Io)?;

        Ok(())
    }

    /// 初始化握手
    pub async fn initialize(&mut self) -> Result<InitializeResult> {
        self.initialize_with_options(json!({})).await
    }

    /// 带自定义选项的初始化握手
    pub async fn initialize_with_options(&mut self, init_options: Value) -> Result<InitializeResult> {
        let root_uri = Url::from_file_path(&self.workspace)
            .map_err(|_| LspError::Protocol("Invalid workspace path".into()))?
            .to_string();

        let mut params = json!({
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "documentSymbol": {
                        "hierarchicalDocumentSymbolSupport": true
                    },
                    "callHierarchy": {
                        "dynamicRegistration": false
                    },
                    "references": {
                        "dynamicRegistration": false
                    }
                }
            }
        });

        // 合并 initializationOptions
        if !init_options.is_null() && init_options != json!({}) {
            params.as_object_mut().unwrap()
                .insert("initializationOptions".to_string(), init_options);
        }

        let result: InitializeResult = self.request("initialize", params).await?;

        self.notify("initialized", json!({}))?;

        Ok(result)
    }

    /// 打开文件
    pub fn open_file(&mut self, path: &str, content: &str, language_id: &str) -> Result<()> {
        let uri = Url::from_file_path(path)
            .map_err(|_| LspError::Protocol("Invalid path".into()))?
            .to_string();

        self.notify("textDocument/didOpen", json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": content
            }
        }))
    }

    /// 获取文档符号
    pub async fn document_symbols(&mut self, path: &str) -> Result<Vec<DocumentSymbol>> {
        let uri = Url::from_file_path(path)
            .map_err(|_| LspError::Protocol("Invalid path".into()))?
            .to_string();

        let result: DocumentSymbolResponse = self.request("textDocument/documentSymbol", json!({
            "textDocument": { "uri": uri }
        })).await?;

        match result {
            DocumentSymbolResponse::Nested(symbols) => Ok(symbols),
            DocumentSymbolResponse::Flat(_) => Ok(vec![]),
        }
    }

    /// 准备调用层次
    pub async fn prepare_call_hierarchy(&mut self, path: &str, line: u32, column: u32) -> Result<Vec<CallHierarchyItem>> {
        let uri = Url::from_file_path(path)
            .map_err(|_| LspError::Protocol("Invalid path".into()))?
            .to_string();

        self.request("textDocument/prepareCallHierarchy", json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": column }
        })).await
    }

    /// 获取调用者
    pub async fn incoming_calls(&mut self, item: &CallHierarchyItem) -> Result<Vec<CallHierarchyIncomingCall>> {
        self.request("callHierarchy/incomingCalls", json!({
            "item": item
        })).await
    }

    /// 获取被调用者
    pub async fn outgoing_calls(&mut self, item: &CallHierarchyItem) -> Result<Vec<CallHierarchyOutgoingCall>> {
        self.request("callHierarchy/outgoingCalls", json!({
            "item": item
        })).await
    }

    /// 获取引用
    pub async fn references(&mut self, path: &str, line: u32, column: u32) -> Result<Vec<Location>> {
        let uri = Url::from_file_path(path)
            .map_err(|_| LspError::Protocol("Invalid path".into()))?
            .to_string();

        self.request("textDocument/references", json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": column },
            "context": { "includeDeclaration": true }
        })).await
    }

    /// 关闭
    pub fn shutdown(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
        }
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
