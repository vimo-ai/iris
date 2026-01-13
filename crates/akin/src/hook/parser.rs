//! 代码解析器 - 使用 tree-sitter 提取代码单元

use lsp::CodeUnit;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::Parser;

/// 支持的代码文件扩展名
const CODE_EXTENSIONS: &[(&str, &str)] = &[
    (".rs", "rust"),
    (".swift", "swift"),
    (".py", "python"),
    (".ts", "typescript"),
    (".tsx", "typescript"),
    (".js", "javascript"),
    (".jsx", "javascript"),
    (".go", "go"),
];

/// 获取数据库路径
pub fn get_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".vimo")
        .join("akin")
        .join("akin.db")
}

/// 检查是否是代码文件
pub fn is_code_file(file_path: &str) -> bool {
    let path = Path::new(file_path);
    if let Some(ext) = path.extension() {
        let ext_str = format!(".{}", ext.to_string_lossy().to_lowercase());
        CODE_EXTENSIONS.iter().any(|(e, _)| *e == ext_str)
    } else {
        false
    }
}

/// 获取文件语言
pub fn get_language(file_path: &str) -> Option<&'static str> {
    let path = Path::new(file_path);
    if let Some(ext) = path.extension() {
        let ext_str = format!(".{}", ext.to_string_lossy().to_lowercase());
        CODE_EXTENSIONS.iter()
            .find(|(e, _)| *e == ext_str)
            .map(|(_, lang)| *lang)
    } else {
        None
    }
}

/// 代码解析器
pub struct CodeParser {
    rust_parser: Option<Parser>,
    swift_parser: Option<Parser>,
}

impl CodeParser {
    pub fn new() -> Self {
        Self {
            rust_parser: Self::create_rust_parser(),
            swift_parser: Self::create_swift_parser(),
        }
    }

    fn create_rust_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::language();
        parser.set_language(&language).ok()?;
        Some(parser)
    }

    fn create_swift_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        let language = tree_sitter_swift::language();
        parser.set_language(&language).ok()?;
        Some(parser)
    }

    /// 从代码中提取函数
    pub fn extract_functions(&mut self, content: &str, file_path: &str, min_lines: u32) -> Vec<CodeUnit> {
        let lang = match get_language(file_path) {
            Some(l) => l,
            None => return vec![],
        };

        match lang {
            "rust" => self.extract_rust_functions(content, file_path, min_lines),
            "swift" => self.extract_swift_functions(content, file_path, min_lines),
            _ => vec![],
        }
    }

    fn extract_rust_functions(&mut self, content: &str, file_path: &str, min_lines: u32) -> Vec<CodeUnit> {
        let parser = match &mut self.rust_parser {
            Some(p) => p,
            None => return vec![],
        };

        let tree = match parser.parse(content.as_bytes(), None) {
            Some(t) => t,
            None => return vec![],
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut units = Vec::new();

        // 第一遍: 提取所有 struct 的字段定义
        let struct_fields = Self::extract_rust_struct_fields(tree.root_node(), content);

        // 第二遍: 提取函数，关联 struct 字段作为上下文
        Self::visit_rust_node(
            tree.root_node(),
            content,
            &lines,
            file_path,
            min_lines,
            None,
            &struct_fields,
            &mut units,
        );

        units
    }

    /// 提取所有 Rust struct 的字段定义
    fn extract_rust_struct_fields(node: tree_sitter::Node, content: &str) -> HashMap<String, Vec<String>> {
        let mut struct_fields: HashMap<String, Vec<String>> = HashMap::new();

        fn collect_structs(node: tree_sitter::Node, content: &str, fields: &mut HashMap<String, Vec<String>>) {
            if node.kind() == "struct_item" {
                // 获取 struct 名称
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "type_identifier")
                    .map(|c| content[c.byte_range()].to_string());

                if let Some(struct_name) = name {
                    // 提取字段列表
                    let mut field_list = Vec::new();
                    for child in node.children(&mut node.walk()) {
                        if child.kind() == "field_declaration_list" {
                            for field in child.children(&mut child.walk()) {
                                if field.kind() == "field_declaration" {
                                    let field_text = &content[field.byte_range()];
                                    field_list.push(field_text.to_string());
                                }
                            }
                        }
                    }
                    if !field_list.is_empty() {
                        fields.insert(struct_name, field_list);
                    }
                }
            }

            // 递归处理子节点
            for child in node.children(&mut node.walk()) {
                collect_structs(child, content, fields);
            }
        }

        collect_structs(node, content, &mut struct_fields);
        struct_fields
    }

    fn visit_rust_node(
        node: tree_sitter::Node,
        content: &str,
        lines: &[&str],
        file_path: &str,
        min_lines: u32,
        impl_name: Option<&str>,
        struct_fields: &HashMap<String, Vec<String>>,
        units: &mut Vec<CodeUnit>,
    ) {
        if node.kind() == "function_item" {
            let start_line = node.start_position().row;
            let end_line = node.end_position().row + 1;

            if (end_line - start_line) as u32 >= min_lines {
                let mut body = lines[start_line..end_line].join("\n");

                // 如果在 impl 块中，尝试获取 struct 字段作为上下文
                if let Some(impl_n) = impl_name {
                    if let Some(fields) = struct_fields.get(impl_n) {
                        if !fields.is_empty() {
                            let fields_context = format!("// Struct fields:\n{}\n\n", fields.join("\n"));
                            body = fields_context + &body;
                        }
                    }
                }

                // 获取函数名
                let func_name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "identifier")
                    .map(|c| &content[c.byte_range()])
                    .unwrap_or("unknown");

                let qualified_name = if let Some(impl_n) = impl_name {
                    format!("rust:{}::{}::{}", file_path, impl_n, func_name)
                } else {
                    format!("rust::{}::{}", file_path, func_name)
                };

                units.push(CodeUnit {
                    qualified_name,
                    file_path: file_path.to_string(),
                    kind: if impl_name.is_some() { "method" } else { "function" }.to_string(),
                    range_start: start_line as u32 + 1,
                    range_end: end_line as u32,
                    body,
                    selection_line: start_line as u32 + 1,
                    selection_column: 0,
                });
            }
        } else if node.kind() == "impl_item" {
            // 获取 impl 的类型名
            let type_name = node.children(&mut node.walk())
                .find(|c| c.kind() == "type_identifier")
                .map(|c| content[c.byte_range()].to_string());

            // 递归处理 impl 体
            for child in node.children(&mut node.walk()) {
                if child.kind() == "declaration_list" {
                    for member in child.children(&mut child.walk()) {
                        Self::visit_rust_node(
                            member,
                            content,
                            lines,
                            file_path,
                            min_lines,
                            type_name.as_deref(),
                            struct_fields,
                            units,
                        );
                    }
                }
            }
        } else {
            // 递归处理其他节点
            for child in node.children(&mut node.walk()) {
                Self::visit_rust_node(child, content, lines, file_path, min_lines, impl_name, struct_fields, units);
            }
        }
    }

    fn extract_swift_functions(&mut self, content: &str, file_path: &str, min_lines: u32) -> Vec<CodeUnit> {
        let parser = match &mut self.swift_parser {
            Some(p) => p,
            None => return vec![],
        };

        let tree = match parser.parse(content.as_bytes(), None) {
            Some(t) => t,
            None => return vec![],
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut units = Vec::new();

        Self::visit_swift_node(
            tree.root_node(),
            content,
            &lines,
            file_path,
            min_lines,
            None,
            None, // class_properties
            &mut units,
        );

        units
    }

    /// 从 Swift class_body 提取属性声明
    fn extract_swift_properties(class_body: tree_sitter::Node, content: &str) -> Vec<String> {
        let mut properties = Vec::new();
        for child in class_body.children(&mut class_body.walk()) {
            if child.kind() == "property_declaration" {
                let prop_text = &content[child.byte_range()];
                properties.push(prop_text.to_string());
            }
        }
        properties
    }

    fn visit_swift_node(
        node: tree_sitter::Node,
        content: &str,
        lines: &[&str],
        file_path: &str,
        min_lines: u32,
        class_name: Option<&str>,
        class_properties: Option<&[String]>,
        units: &mut Vec<CodeUnit>,
    ) {
        let kind = node.kind();

        // Swift 函数定义
        if kind == "function_declaration" || kind == "subscript_declaration" || kind == "init_declaration" {
            let start_line = node.start_position().row;
            let end_line = node.end_position().row + 1;

            if (end_line - start_line) as u32 >= min_lines {
                let mut body = lines[start_line..end_line].join("\n");

                // 如果有类属性，附加到 body 前面作为上下文
                if let Some(props) = class_properties {
                    if !props.is_empty() {
                        let props_context = format!("// Class properties:\n{}\n\n", props.join("\n"));
                        body = props_context + &body;
                    }
                }

                // 获取函数名
                let func_name = if kind == "init_declaration" {
                    "init".to_string()
                } else {
                    node.children(&mut node.walk())
                        .find(|c| c.kind() == "simple_identifier")
                        .map(|c| content[c.byte_range()].to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                };

                let qualified_name = if let Some(class_n) = class_name {
                    format!("swift:{}::{}::{}", file_path, class_n, func_name)
                } else {
                    format!("swift:{}::{}", file_path, func_name)
                };

                units.push(CodeUnit {
                    qualified_name,
                    file_path: file_path.to_string(),
                    kind: if class_name.is_some() { "method" } else { "function" }.to_string(),
                    range_start: start_line as u32 + 1,
                    range_end: end_line as u32,
                    body,
                    selection_line: start_line as u32 + 1,
                    selection_column: 0,
                });
            }
        }
        // 类/结构体/扩展
        else if kind == "class_declaration" || kind == "struct_declaration"
            || kind == "extension_declaration" || kind == "enum_declaration"
        {
            // 获取类名
            let name = node.children(&mut node.walk())
                .find(|c| c.kind() == "type_identifier" || c.kind() == "simple_identifier")
                .map(|c| content[c.byte_range()].to_string());

            // 递归处理类体
            for child in node.children(&mut node.walk()) {
                if child.kind() == "class_body" {
                    // 提取属性作为上下文
                    let props = Self::extract_swift_properties(child, content);
                    for member in child.children(&mut child.walk()) {
                        Self::visit_swift_node(
                            member,
                            content,
                            lines,
                            file_path,
                            min_lines,
                            name.as_deref(),
                            Some(&props),
                            units,
                        );
                    }
                }
            }
        } else {
            // 递归处理其他节点
            for child in node.children(&mut node.walk()) {
                Self::visit_swift_node(child, content, lines, file_path, min_lines, class_name, class_properties, units);
            }
        }
    }
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_code_file() {
        assert!(is_code_file("foo.rs"));
        assert!(is_code_file("bar.swift"));
        assert!(is_code_file("baz.py"));
        assert!(is_code_file("test.ts"));
        assert!(!is_code_file("readme.md"));
        assert!(!is_code_file("config.json"));
    }

    #[test]
    fn test_get_language() {
        assert_eq!(get_language("foo.rs"), Some("rust"));
        assert_eq!(get_language("bar.swift"), Some("swift"));
        assert_eq!(get_language("baz.py"), Some("python"));
        assert_eq!(get_language("readme.md"), None);
    }

    #[test]
    fn test_extract_rust_functions() {
        let mut parser = CodeParser::new();

        let content = r#"fn foo() {
    let x = 1;
    let y = 2;
    let z = 3;
    println!("{}", x + y + z);
}

impl Bar {
    fn bar_method(&self) {
        let a = 1;
        let b = 2;
        let c = 3;
        let d = 4;
        println!("{}", a + b + c + d);
    }
}"#;
        let units = parser.extract_functions(content, "test.rs", 5);
        assert_eq!(units.len(), 2, "Expected 2 functions, found {:?}", units.iter().map(|u| &u.qualified_name).collect::<Vec<_>>());
        assert!(units[0].qualified_name.contains("foo"));
        assert!(units[1].qualified_name.contains("bar_method"));
    }

    #[test]
    fn test_extract_swift_functions() {
        let mut parser = CodeParser::new();
        let content = r#"
func foo() {
    let x = 1
    let y = 2
    let z = 3
    print(x + y + z)
}

class Bar {
    func barMethod() {
        let a = 1
        let b = 2
        let c = 3
        let d = 4
        print(a + b + c + d)
    }
}
"#;
        let units = parser.extract_functions(content, "test.swift", 5);
        assert_eq!(units.len(), 2);
        assert!(units[0].qualified_name.contains("foo"));
        assert!(units[1].qualified_name.contains("barMethod"));
    }

    #[test]
    fn test_extract_rust_functions_with_struct_fields() {
        let mut parser = CodeParser::new();

        let content = r#"
struct Session {
    id: String,
    user_id: String,
    created_at: i64,
}

impl Session {
    fn new(user_id: String) -> Self {
        Self {
            id: uuid(),
            user_id,
            created_at: now(),
        }
    }

    fn is_valid(&self) -> bool {
        self.created_at > 0
            && !self.id.is_empty()
            && !self.user_id.is_empty()
    }
}
"#;
        let units = parser.extract_functions(content, "test.rs", 5);
        assert_eq!(units.len(), 2, "Expected 2 methods, found {:?}", units.iter().map(|u| &u.qualified_name).collect::<Vec<_>>());

        // 验证方法包含 struct 字段上下文
        let new_method = &units[0];
        assert!(new_method.qualified_name.contains("new"));
        assert!(new_method.body.contains("// Struct fields:"), "Method body should contain struct fields context");
        assert!(new_method.body.contains("id: String"), "Method body should contain 'id' field");
        assert!(new_method.body.contains("user_id: String"), "Method body should contain 'user_id' field");

        let is_valid_method = &units[1];
        assert!(is_valid_method.qualified_name.contains("is_valid"));
        assert!(is_valid_method.body.contains("// Struct fields:"));
    }

    #[test]
    fn test_extract_swift_functions_with_properties() {
        let mut parser = CodeParser::new();

        let content = r#"
class SessionManager {
    var sessionId: String = ""
    var userId: String = ""
    private var createdAt: Date = Date()

    func establish(userId: String) {
        self.sessionId = UUID().uuidString
        self.userId = userId
        self.createdAt = Date()
    }

    func isValid() -> Bool {
        return !sessionId.isEmpty
            && !userId.isEmpty
            && createdAt.timeIntervalSinceNow < 3600
    }
}
"#;
        let units = parser.extract_functions(content, "test.swift", 5);
        assert_eq!(units.len(), 2, "Expected 2 methods, found {:?}", units.iter().map(|u| &u.qualified_name).collect::<Vec<_>>());

        // 验证方法包含类属性上下文
        let establish_method = &units[0];
        assert!(establish_method.qualified_name.contains("establish"));
        assert!(establish_method.body.contains("// Class properties:"), "Method body should contain class properties context");
        assert!(establish_method.body.contains("sessionId"), "Method body should contain 'sessionId' property");
        assert!(establish_method.body.contains("userId"), "Method body should contain 'userId' property");

        let is_valid_method = &units[1];
        assert!(is_valid_method.qualified_name.contains("isValid"));
        assert!(is_valid_method.body.contains("// Class properties:"));
    }

    #[test]
    fn test_real_swift_file_property_context() {
        let swift_path = "/Users/higuaifan/Desktop/vimo/ETerm/ETerm/Packages/PanelLayoutKit/Sources/PanelLayoutKit/Session/DragSession.swift";

        let content = match std::fs::read_to_string(swift_path) {
            Ok(c) => c,
            Err(_) => {
                println!("跳过测试：文件不存在 {}", swift_path);
                return;
            }
        };

        let mut parser = CodeParser::new();
        let units = parser.extract_functions(&content, "DragSession.swift", 5);

        println!("\n=== 真实 Swift 文件属性上下文测试 ===");
        println!("文件: {}", swift_path);
        println!("提取到 {} 个代码单元:\n", units.len());

        let mut has_property_context = false;
        for (i, unit) in units.iter().enumerate() {
            println!("--- [{}/{}] {} ---", i + 1, units.len(), unit.qualified_name);
            println!("类型: {}, 行号: {}-{}", unit.kind, unit.range_start, unit.range_end);

            if unit.body.contains("// Class properties:") {
                has_property_context = true;
                println!("✅ 包含属性上下文");
                // 显示属性上下文部分
                let lines: Vec<&str> = unit.body.lines().collect();
                let context_end = lines.iter().position(|l| !l.starts_with("//") && !l.trim().starts_with("var") && !l.trim().starts_with("let") && !l.trim().starts_with("private") && !l.trim().is_empty()).unwrap_or(10);
                println!("属性上下文:\n{}", lines[..context_end.min(15)].join("\n"));
            } else {
                println!("⚠️ 无属性上下文（可能是顶级函数）");
            }
            println!();
        }

        assert!(has_property_context, "至少一个方法应该包含属性上下文");
    }

    #[test]
    fn test_real_rust_file_struct_fields() {
        let rust_path = "/Users/higuaifan/Desktop/vimo/iris/crates/akin/src/scanner.rs";

        let content = match std::fs::read_to_string(rust_path) {
            Ok(c) => c,
            Err(_) => {
                println!("跳过测试：文件不存在 {}", rust_path);
                return;
            }
        };

        let mut parser = CodeParser::new();
        let units = parser.extract_functions(&content, "scanner.rs", 5);

        println!("\n=== 真实 Rust 文件 struct 字段上下文测试 ===");
        println!("文件: {}", rust_path);
        println!("提取到 {} 个代码单元:\n", units.len());

        let mut has_struct_context = false;
        for (i, unit) in units.iter().enumerate() {
            println!("--- [{}/{}] {} ---", i + 1, units.len(), unit.qualified_name);
            println!("类型: {}, 行号: {}-{}", unit.kind, unit.range_start, unit.range_end);

            if unit.body.contains("// Struct fields:") {
                has_struct_context = true;
                println!("✅ 包含 struct 字段上下文");
                // 显示字段上下文部分
                let lines: Vec<&str> = unit.body.lines().collect();
                let context_end = lines.iter().position(|l| {
                    let trimmed = l.trim();
                    !l.starts_with("//") && !trimmed.is_empty() && !trimmed.contains(": ")
                }).unwrap_or(10);
                println!("字段上下文:\n{}", lines[..context_end.min(10)].join("\n"));
            } else {
                println!("⚠️ 无 struct 字段上下文（可能是顶级函数）");
            }
            println!();
        }

        assert!(has_struct_context, "至少一个方法应该包含 struct 字段上下文");
    }
}
