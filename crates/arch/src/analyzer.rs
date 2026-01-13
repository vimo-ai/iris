use lsp::{FunctionNode, FunctionRef, LanguageAdapter};
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
    /// 函数映射: (file_path, line) -> FunctionNode
    functions: HashMap<FunctionRef, FunctionNode>,
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
            let key = FunctionRef::new(unit.file_path.clone(), unit.selection_line);

            let hierarchy = adapter
                .get_call_hierarchy(unit)
                .await
                .map_err(|e| ArchError::Lsp(e.to_string()))?;

            // 直接使用 FunctionRef，无需格式转换
            let callers: Vec<FunctionRef> = hierarchy
                .incoming
                .iter()
                .map(|c| c.as_ref())
                .collect();

            let callees: Vec<FunctionRef> = hierarchy
                .outgoing
                .iter()
                .map(|c| c.as_ref())
                .collect();

            // 提取短名字用于显示
            let short_name = unit.qualified_name
                .split("::")
                .last()
                .unwrap_or(&unit.qualified_name)
                .to_string();

            let node = FunctionNode {
                file_path: unit.file_path.clone(),
                line: unit.selection_line,
                name: short_name,
                callers,
                callees,
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

        // 查找起始节点
        let start_ref = self.find_function_ref(root);
        if let Some(func_ref) = start_ref {
            self.build_tree(&func_ref, direction, 0, max_depth, &mut visited, &mut result);
        }
        result
    }

    /// 通过名字查找函数引用
    fn find_function_ref(&self, name: &str) -> Option<FunctionRef> {
        // 精确匹配短名字
        self.functions.iter()
            .find(|(_, node)| node.name == name)
            .map(|(k, _)| k.clone())
            // 或者后缀匹配
            .or_else(|| {
                self.functions.iter()
                    .find(|(_, node)| node.name.ends_with(&format!("::{}", name)))
                    .map(|(k, _)| k.clone())
            })
    }

    fn build_tree(
        &self,
        func_ref: &FunctionRef,
        direction: CallDirection,
        depth: usize,
        max_depth: usize,
        visited: &mut std::collections::HashSet<FunctionRef>,
        result: &mut Vec<CallTreeNode>,
    ) {
        if depth > max_depth || visited.contains(func_ref) {
            return;
        }
        visited.insert(func_ref.clone());

        if let Some(node) = self.functions.get(func_ref) {
            result.push(CallTreeNode {
                name: node.name.clone(),
                file_path: node.file_path.clone(),
                line: node.line,
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
    pub fn functions(&self) -> &HashMap<FunctionRef, FunctionNode> {
        &self.functions
    }

    /// 添加函数节点 (用于测试)
    #[doc(hidden)]
    pub fn add_function(&mut self, file_path: &str, line: u32, node: FunctionNode) {
        let key = FunctionRef::new(file_path.to_string(), line);
        self.functions.insert(key, node);
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
    pub file_path: String,
    pub line: u32,
    pub depth: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(name: &str, callers: Vec<(&str, u32)>, callees: Vec<(&str, u32)>) -> FunctionNode {
        FunctionNode {
            file_path: "/test/file.rs".to_string(),
            line: 1,
            name: name.to_string(),
            callers: callers.into_iter().map(|(f, l)| FunctionRef::new(f.to_string(), l)).collect(),
            callees: callees.into_iter().map(|(f, l)| FunctionRef::new(f.to_string(), l)).collect(),
        }
    }

    #[test]
    fn test_is_entry_point_main() {
        let node = make_node("main", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_test() {
        let node = make_node("test_something", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_new() {
        let node = make_node("new", vec![], vec![]);
        assert!(ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_is_entry_point_regular_function() {
        let node = make_node("helper_function", vec![], vec![]);
        assert!(!ArchitectureAnalyzer::is_entry_point(&node));
    }

    #[test]
    fn test_find_dead_code_no_callers() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut foo = make_node("foo", vec![], vec![]);
        foo.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        analyzer.add_function("/test/file.rs", 1, foo);

        let mut bar = make_node("bar", vec![], vec![]);
        bar.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 2, bar);

        analyzer.add_function("/test/file.rs", 3, make_node("unused", vec![], vec![]));

        let dead = analyzer.find_dead_code();
        assert_eq!(dead.len(), 2); // foo and unused have no callers
        let names: Vec<_> = dead.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"unused"));
    }

    #[test]
    fn test_find_dead_code_excludes_entry_points() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut main_node = make_node("main", vec![], vec![]);
        main_node.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        analyzer.add_function("/test/file.rs", 1, main_node);

        let mut foo = make_node("foo", vec![], vec![]);
        foo.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 2, foo);

        let dead = analyzer.find_dead_code();
        assert!(dead.is_empty()); // main is entry point, foo has caller
    }

    #[test]
    fn test_get_call_tree_outgoing() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut root = make_node("root", vec![], vec![]);
        root.callees = vec![
            FunctionRef::new("/test/file.rs".to_string(), 2),
            FunctionRef::new("/test/file.rs".to_string(), 3),
        ];
        analyzer.add_function("/test/file.rs", 1, root);

        let mut child1 = make_node("child1", vec![], vec![]);
        child1.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        child1.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 4)];
        analyzer.add_function("/test/file.rs", 2, child1);

        let mut child2 = make_node("child2", vec![], vec![]);
        child2.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 3, child2);

        let mut grandchild = make_node("grandchild", vec![], vec![]);
        grandchild.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        analyzer.add_function("/test/file.rs", 4, grandchild);

        let tree = analyzer.get_call_tree("root", CallDirection::Outgoing, 3);

        assert_eq!(tree.len(), 4);
        assert_eq!(tree[0].name, "root");
        assert_eq!(tree[0].depth, 0);
    }

    #[test]
    fn test_get_call_tree_incoming() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut root = make_node("root", vec![], vec![]);
        root.callers = vec![
            FunctionRef::new("/test/file.rs".to_string(), 2),
            FunctionRef::new("/test/file.rs".to_string(), 3),
        ];
        analyzer.add_function("/test/file.rs", 1, root);

        let mut caller1 = make_node("caller1", vec![], vec![]);
        caller1.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 2, caller1);

        let mut caller2 = make_node("caller2", vec![], vec![]);
        caller2.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 3, caller2);

        let tree = analyzer.get_call_tree("root", CallDirection::Incoming, 2);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].name, "root");
    }

    #[test]
    fn test_get_call_tree_max_depth() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut a = make_node("a", vec![], vec![]);
        a.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        analyzer.add_function("/test/file.rs", 1, a);

        let mut b = make_node("b", vec![], vec![]);
        b.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        b.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 3)];
        analyzer.add_function("/test/file.rs", 2, b);

        let mut c = make_node("c", vec![], vec![]);
        c.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        c.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 4)];
        analyzer.add_function("/test/file.rs", 3, c);

        let mut d = make_node("d", vec![], vec![]);
        d.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 3)];
        analyzer.add_function("/test/file.rs", 4, d);

        let tree = analyzer.get_call_tree("a", CallDirection::Outgoing, 1);

        // depth 0: a, depth 1: b, depth 2: c (not included)
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn test_get_call_tree_handles_cycles() {
        let mut analyzer = ArchitectureAnalyzer::new();

        let mut a = make_node("a", vec![], vec![]);
        a.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        a.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 2)];
        analyzer.add_function("/test/file.rs", 1, a);

        let mut b = make_node("b", vec![], vec![]);
        b.callers = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        b.callees = vec![FunctionRef::new("/test/file.rs".to_string(), 1)];
        analyzer.add_function("/test/file.rs", 2, b);

        let tree = analyzer.get_call_tree("a", CallDirection::Outgoing, 10);

        // Should not infinite loop
        assert_eq!(tree.len(), 2);
    }
}
