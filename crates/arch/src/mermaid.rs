use crate::analyzer::ArchitectureAnalyzer;
use lsp::FunctionNode;
use std::collections::HashMap;
use std::path::Path;

/// Mermaid 图生成器
pub struct MermaidGenerator {
    max_nodes: usize,
}

impl MermaidGenerator {
    pub fn new() -> Self {
        Self { max_nodes: 100 }
    }

    pub fn with_max_nodes(mut self, max: usize) -> Self {
        self.max_nodes = max;
        self
    }

    /// 生成调用图 Mermaid 代码
    pub fn generate_call_graph(&self, analyzer: &ArchitectureAnalyzer) -> String {
        let functions = analyzer.functions();
        let mut lines = vec!["flowchart TD".to_string()];

        // 按连接数排序，取前 N 个
        let mut sorted: Vec<_> = functions.values().collect();
        sorted.sort_by_key(|n| std::cmp::Reverse(n.callers.len() + n.callees.len()));
        sorted.truncate(self.max_nodes);

        let included: std::collections::HashSet<_> = sorted.iter().map(|n| &n.name).collect();

        // 生成节点
        for node in &sorted {
            let short_name = Self::short_name(&node.name);
            let style = if node.callers.is_empty() {
                format!("    {}[[{}]]", Self::node_id(&node.name), short_name)
            } else if node.callees.is_empty() {
                format!("    {}([{}])", Self::node_id(&node.name), short_name)
            } else {
                format!("    {}[{}]", Self::node_id(&node.name), short_name)
            };
            lines.push(style);
        }

        // 生成边
        for node in &sorted {
            for callee in &node.callees {
                if included.contains(callee) {
                    lines.push(format!(
                        "    {} --> {}",
                        Self::node_id(&node.name),
                        Self::node_id(callee)
                    ));
                }
            }
        }

        lines.join("\n")
    }

    /// 生成模块依赖图
    pub fn generate_module_diagram(&self, analyzer: &ArchitectureAnalyzer, workspace: &str) -> String {
        let functions = analyzer.functions();
        let mut lines = vec!["flowchart TD".to_string()];

        // 按文件分组
        let mut modules: HashMap<String, Vec<&FunctionNode>> = HashMap::new();
        for node in functions.values() {
            let module = Self::extract_module(&node.file_path, workspace);
            modules.entry(module).or_default().push(node);
        }

        // 计算跨模块调用
        let mut edges: HashMap<(String, String), usize> = HashMap::new();
        for node in functions.values() {
            let from_module = Self::extract_module(&node.file_path, workspace);
            for callee in &node.callees {
                if let Some(callee_node) = functions.values().find(|n| n.name == *callee) {
                    let to_module = Self::extract_module(&callee_node.file_path, workspace);
                    if from_module != to_module {
                        *edges.entry((from_module.clone(), to_module)).or_insert(0) += 1;
                    }
                }
            }
        }

        // 生成模块节点
        for module in modules.keys() {
            let id = Self::node_id(module);
            lines.push(format!("    {}[{}]", id, module));
        }

        // 生成边 (带权重)
        for ((from, to), count) in edges {
            lines.push(format!(
                "    {} -->|{}| {}",
                Self::node_id(&from),
                count,
                Self::node_id(&to)
            ));
        }

        lines.join("\n")
    }

    #[doc(hidden)]
    pub fn node_id(name: &str) -> String {
        name.replace("::", "_")
            .replace("/", "_")
            .replace(".", "_")
            .replace("-", "_")
    }

    #[doc(hidden)]
    pub fn short_name(name: &str) -> String {
        name.split("::").last().unwrap_or(name).to_string()
    }

    #[doc(hidden)]
    pub fn extract_module(file_path: &str, workspace: &str) -> String {
        let relative = file_path
            .strip_prefix(workspace)
            .unwrap_or(file_path)
            .trim_start_matches('/');
        let path = Path::new(relative);

        // 使用相对路径（去掉扩展名）作为模块名，避免同名文件冲突
        path.with_extension("")
            .to_str()
            .unwrap_or("unknown")
            .replace('/', "::")
    }
}

impl Default for MermaidGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id_replaces_special_chars() {
        assert_eq!(MermaidGenerator::node_id("foo::bar"), "foo_bar");
        assert_eq!(MermaidGenerator::node_id("src/lib.rs"), "src_lib_rs");
        assert_eq!(MermaidGenerator::node_id("my-crate"), "my_crate");
        assert_eq!(MermaidGenerator::node_id("a::b/c.d-e"), "a_b_c_d_e");
    }

    #[test]
    fn test_short_name_extracts_last_segment() {
        assert_eq!(MermaidGenerator::short_name("foo::bar::baz"), "baz");
        assert_eq!(MermaidGenerator::short_name("single"), "single");
        assert_eq!(MermaidGenerator::short_name("a::b"), "b");
    }

    #[test]
    fn test_extract_module_strips_workspace() {
        let result = MermaidGenerator::extract_module(
            "/workspace/src/lib.rs",
            "/workspace"
        );
        assert_eq!(result, "src::lib");
    }

    #[test]
    fn test_extract_module_handles_nested_paths() {
        let result = MermaidGenerator::extract_module(
            "/workspace/crates/lsp/src/protocol.rs",
            "/workspace"
        );
        assert_eq!(result, "crates::lsp::src::protocol");
    }

    #[test]
    fn test_extract_module_no_workspace_prefix() {
        let result = MermaidGenerator::extract_module(
            "/other/path/file.rs",
            "/workspace"
        );
        assert_eq!(result, "other::path::file");
    }

    #[test]
    fn test_extract_module_avoids_collision() {
        // 不同目录下同名文件应该有不同的模块名
        let mod1 = MermaidGenerator::extract_module("/ws/a/lib.rs", "/ws");
        let mod2 = MermaidGenerator::extract_module("/ws/b/lib.rs", "/ws");
        assert_ne!(mod1, mod2);
        assert_eq!(mod1, "a::lib");
        assert_eq!(mod2, "b::lib");
    }

    #[test]
    fn test_generator_builder() {
        let gen = MermaidGenerator::new().with_max_nodes(50);
        assert_eq!(gen.max_nodes, 50);
    }

    #[test]
    fn test_generator_default() {
        let gen = MermaidGenerator::default();
        assert_eq!(gen.max_nodes, 100);
    }
}
