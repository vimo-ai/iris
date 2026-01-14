mod rust;
mod swift;
mod typescript;

pub use rust::RustAdapter;
pub use swift::SwiftAdapter;
pub use typescript::TypeScriptAdapter;

use crate::types::{CodeUnit, CallHierarchy};
use crate::protocol::Result;
use async_trait::async_trait;

/// 语言适配器 trait
#[async_trait]
pub trait LanguageAdapter: Send + Sync {
    /// 启动 LSP 服务器
    async fn start(&mut self) -> Result<()>;

    /// 获取所有函数
    async fn get_functions(&mut self) -> Result<Vec<CodeUnit>>;

    /// 获取源文件列表
    fn get_source_files(&self) -> Result<Vec<String>>;

    /// 获取调用层次
    async fn get_call_hierarchy(&mut self, unit: &CodeUnit) -> Result<CallHierarchy>;

    /// 停止
    fn stop(&mut self) -> Result<()>;
}
