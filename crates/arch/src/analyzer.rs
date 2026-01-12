use lsp::{FunctionNode, LanguageAdapter};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArchError {
    #[error("LSP error: {0}")]
    Lsp(String),
}

pub type Result<T> = std::result::Result<T, ArchError>;

/// 架构分析器
pub struct ArchitectureAnalyzer {
    functions: HashMap<String, FunctionNode>,
}

impl ArchitectureAnalyzer {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }

    /// 构建调用图
    pub async fn build_call_graph<A: LanguageAdapter>(
        &mut self,
        adapter: &mut A,
    ) -> Result<()> {
        let units = adapter
            .get_functions()
            .await
            .map_err(|e| ArchError::Lsp(e.to_string()))?;

        for unit in &units {
            let key = format!(
                "{}:{}:{}",
                unit.file_path,
                unit.selection_line,
                unit.qualified_name.split("::").last().unwrap_or(&unit.qualified_name)
            );

            let hierarchy = adapter
                .get_call_hierarchy(unit)
                .await
                .map_err(|e| ArchError::Lsp(e.to_string()))?;

            let node = FunctionNode {
                name: unit.qualified_name.clone(),
                file_path: unit.file_path.clone(),
                line: unit.selection_line,
                callers: hierarchy
                    .incoming
                    .iter()
                    .map(|c| c.stable_id())
                    .collect(),
                callees: hierarchy
                    .outgoing
                    .iter()
                    .map(|c| c.stable_id())
                    .collect(),
            };

            self.functions.insert(key, node);
        }

        Ok(())
    }

    /// 检测死代码 (无调用者的函数)
    pub fn find_dead_code(&self) -> Vec<&FunctionNode> {
        self.functions
            .values()
            .filter(|node| {
                node.callers.is_empty() && !Self::is_entry_point(node)
            })
            .collect()
    }

    /// 判断是否是入口点
    #[doc(hidden)]
    pub fn is_entry_point(node: &FunctionNode) -> bool {
        let name_lower = node.name.to_lowercase();
        let entry_patterns = [
            "main",
            "test_",
            "_test",
            "new",
            "default",
            "init",
            "setup",
            "run",
        ];
        entry_patterns.iter().any(|p| name_lower.contains(p))
    }

    /// 获取调用树
    pub fn get_call_tree(&self, root: &str, direction: CallDirection, max_depth: usize) -> Vec<CallTreeNode> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.build_tree(root, direction, 0, max_depth, &mut visited, &mut result);
        result
    }

    fn build_tree(
        &self,
        name: &str,
        direction: CallDirection,
        depth: usize,
        max_depth: usize,
        visited: &mut std::collections::HashSet<String>,
        result: &mut Vec<CallTreeNode>,
    ) {
        if depth > max_depth || visited.contains(name) {
            return;
        }
        visited.insert(name.to_string());

        // 支持通过 key (file:line:name)、qualified_name 或 short name 查找
        let node = self.functions.get(name)
            .or_else(|| self.functions.values().find(|n| n.name == name))
            .or_else(|| self.functions.values().find(|n| n.name.ends_with(&format!("::{}", name))));

        if let Some(node) = node {
            result.push(CallTreeNode {
                name: node.name.clone(),
                depth,
            });

            let children = match direction {
                CallDirection::Incoming => &node.callers,
                CallDirection::Outgoing => &node.callees,
            };

            for child in children {
                self.build_tree(child, direction, depth + 1, max_depth, visited, result);
            }
        }
    }

    /// 获取所有函数
    pub fn functions(&self) -> &HashMap<String, FunctionNode> {
        &self.functions
    }

    /// 添加函数节点 (用于测试)
    #[doc(hidden)]
    pub fn add_function(&mut self, key: &str, node: FunctionNode) {
        self.functions.insert(key.to_string(), node);
    }
}

impl Default for ArchitectureAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone)]
pub struct CallTreeNode {
    pub name: String,
    pub depth: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(name: &str, callers: Vec<&str>, callees: Vec<&str>) -> FunctionNode {
        FunctionNode {
            name: name.to_string(),
            file_path: "/test/file.rs".to_string(),
            line: 1,
            callers: callers.into_iter().map(String::from).collect(),
            callees: callees.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn test_is_entry_point_main() {
        let node = make_node("my_crate::main", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_test() {
        let node = make_node("my_crate::test_something", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_new() {
        let node = make_node("MyStruct::new", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_regular_function() {
        let node = make_node("my_crate::helper_function", vec![], vec![]);
        assert!(!ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_find_dead_code_no_callers() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("foo", vec![], vec!["bar"]));
        analyzer.add_function("k2", make_node("bar", vec!["foo"], vec![]));
        analyzer.add_function("k3", make_node("unused", vec![], vec![]));

        let dead = analyzer.find_dead_code();
        assert_eq!(dead.len(), 2); // foo and unused have no callers
        let names: Vec<_> = dead.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"unused"));
    }

    #[test]
    fn test_find_dead_code_excludes_entry_points() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("main", vec![], vec!["foo"]));
        analyzer.add_function("k2", make_node("foo", vec!["main"], vec![]));

        let dead = analyzer.find_dead_code();
        assert!(dead.is_empty()); // main is entry point, foo has caller
    }

    #[test]
    fn test_get_call_tree_outgoing() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("root", vec![], vec!["child1", "child2"]));
        analyzer.add_function("k2", make_node("child1", vec!["root"], vec!["grandchild"]));
        analyzer.add_function("k3", make_node("child2", vec!["root"], vec![]));
        analyzer.add_function("k4", make_node("grandchild", vec!["child1"], vec![]));

        let tree = analyzer.get_call_tree("root", CallDirection::Outgoing, 3);

        assert_eq!(tree.len(), 4);
        assert_eq!(tree[0].name, "root");
        assert_eq!(tree[0].depth, 0);
    }

    #[test]
    fn test_get_call_tree_incoming() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("root", vec!["caller1", "caller2"], vec![]));
        analyzer.add_function("k2", make_node("caller1", vec![], vec!["root"]));
        analyzer.add_function("k3", make_node("caller2", vec![], vec!["root"]));

        let tree = analyzer.get_call_tree("root", CallDirection::Incoming, 2);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].name, "root");
    }

    #[test]
    fn test_get_call_tree_max_depth() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("a", vec![], vec!["b"]));
        analyzer.add_function("k2", make_node("b", vec!["a"], vec!["c"]));
        analyzer.add_function("k3", make_node("c", vec!["b"], vec!["d"]));
        analyzer.add_function("k4", make_node("d", vec!["c"], vec![]));

        let tree = analyzer.get_call_tree("a", CallDirection::Outgoing, 1);

        // depth 0: a, depth 1: b, depth 2: c (not included)
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn test_get_call_tree_handles_cycles() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("k1", make_node("a", vec!["b"], vec!["b"]));
        analyzer.add_function("k2", make_node("b", vec!["a"], vec!["a"]));

        let tree = analyzer.get_call_tree("a", CallDirection::Outgoing, 10);

        // Should not infinite loop
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn test_get_call_tree_by_qualified_name() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("file:1:foo", make_node("my::module::foo", vec![], vec![]));

        // Should find by qualified name
        let tree = analyzer.get_call_tree("my::module::foo", CallDirection::Outgoing, 1);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "my::module::foo");
    }

    #[test]
    fn test_get_call_tree_by_short_name() {
        let mut analyzer = ArchitectureAnalyzer::new();
        analyzer.add_function("file:1:foo", make_node("my::module::foo", vec![], vec![]));

        // Should find by short name
        let tree = analyzer.get_call_tree("foo", CallDirection::Outgoing, 1);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "my::module::foo");
    }
}
