use serde::{Deserialize, Serialize};

/// 代码单元 - 函数/方法
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeUnit {
    /// 完整限定名: "rust:src/lib.rs::module::Function"
    pub qualified_name: String,
    /// 文件路径
    pub file_path: String,
    /// 类型: "function", "method"
    pub kind: String,
    /// 起始行
    pub range_start: u32,
    /// 结束行
    pub range_end: u32,
    /// 函数体源码
    pub body: String,
    /// 函数名精确位置 - 行
    pub selection_line: u32,
    /// 函数名精确位置 - 列
    pub selection_column: u32,
}

impl CodeUnit {
    /// 内容哈希 (SHA256 前16位)
    pub fn content_hash(&self) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(self.body.as_bytes());
        let result = hasher.finalize();
        format!("{:016x}", u64::from_be_bytes(result[..8].try_into().unwrap()))
    }

    /// 结构哈希 - 规范化后的代码哈希
    pub fn structure_hash(&self) -> String {
        use sha2::{Sha256, Digest};
        let normalized = Self::normalize_code(&self.body);
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        let result = hasher.finalize();
        format!("{:016x}", u64::from_be_bytes(result[..8].try_into().unwrap()))
    }

    /// 规范化代码 - 移除注释、归一化空格、替换字面量
    #[doc(hidden)]
    pub fn normalize_code(code: &str) -> String {
        let mut result = code.to_string();

        // 移除单行注释
        result = result.lines()
            .map(|line| {
                if let Some(pos) = line.find("//") {
                    &line[..pos]
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        // 移除多行注释 (简化处理)
        while let Some(start) = result.find("/*") {
            if let Some(end) = result[start..].find("*/") {
                result = format!("{}{}", &result[..start], &result[start + end + 2..]);
            } else {
                break;
            }
        }

        // 归一化空格
        result = result.split_whitespace().collect::<Vec<_>>().join(" ");

        // 替换字符串字面量
        result = regex_replace_strings(&result);

        // 替换数字
        result = regex_replace_numbers(&result);

        result
    }
}

fn regex_replace_strings(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '"' {
            result.push_str("\"$STR\"");
            // 跳过字符串内容
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '"' { break; }
                if nc == '\\' { chars.next(); }
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn regex_replace_numbers(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            result.push_str("$NUM");
            while let Some(&nc) = chars.peek() {
                if nc.is_ascii_digit() || nc == '.' {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 函数节点 - 用于调用图
#[derive(Debug, Clone)]
pub struct FunctionNode {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
}

/// 调用层次
#[derive(Debug, Clone)]
pub struct CallHierarchy {
    pub incoming: Vec<CallHierarchyItem>,
    pub outgoing: Vec<CallHierarchyItem>,
}

#[derive(Debug, Clone)]
pub struct CallHierarchyItem {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
}

impl CallHierarchyItem {
    /// 唯一标识符 (file:line:name)
    pub fn stable_id(&self) -> String {
        format!("{}:{}:{}", self.file_path, self.line, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_unit(body: &str) -> CodeUnit {
        CodeUnit {
            qualified_name: "test::func".to_string(),
            file_path: "test.rs".to_string(),
            kind: "function".to_string(),
            range_start: 0,
            range_end: 10,
            body: body.to_string(),
            selection_line: 0,
            selection_column: 0,
        }
    }

    #[test]
    fn test_content_hash_deterministic() {
        let unit = make_unit("fn foo() { 42 }");
        let hash1 = unit.content_hash();
        let hash2 = unit.content_hash();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16); // 16 hex chars
    }

    #[test]
    fn test_content_hash_different_for_different_code() {
        let unit1 = make_unit("fn foo() { 42 }");
        let unit2 = make_unit("fn bar() { 42 }");
        assert_ne!(unit1.content_hash(), unit2.content_hash());
    }

    #[test]
    fn test_structure_hash_ignores_comments() {
        let unit1 = make_unit("fn foo() { x + 1 }");
        let unit2 = make_unit("fn foo() { x + 1 } // comment");
        assert_eq!(unit1.structure_hash(), unit2.structure_hash());
    }

    #[test]
    fn test_structure_hash_ignores_multiline_comments() {
        let unit1 = make_unit("fn foo() { x }");
        let unit2 = make_unit("fn foo() { /* comment */ x }");
        assert_eq!(unit1.structure_hash(), unit2.structure_hash());
    }

    #[test]
    fn test_structure_hash_normalizes_whitespace() {
        // 多余空格和换行会被归一化
        let unit1 = make_unit("fn foo() { x + 1 }");
        let unit2 = make_unit("fn foo()  {\n    x   +   1\n}");
        assert_eq!(unit1.structure_hash(), unit2.structure_hash());
    }

    #[test]
    fn test_structure_hash_normalizes_strings() {
        let unit1 = make_unit(r#"println!("hello")"#);
        let unit2 = make_unit(r#"println!("world")"#);
        assert_eq!(unit1.structure_hash(), unit2.structure_hash());
    }

    #[test]
    fn test_structure_hash_normalizes_numbers() {
        let unit1 = make_unit("let x = 42;");
        let unit2 = make_unit("let x = 100;");
        assert_eq!(unit1.structure_hash(), unit2.structure_hash());
    }

    #[test]
    fn test_normalize_code_removes_comments() {
        let code = "let x = 1; // comment\nlet y = 2;";
        let normalized = CodeUnit::normalize_code(code);
        assert!(!normalized.contains("comment"));
        assert!(normalized.contains("let x"));
        assert!(normalized.contains("let y"));
    }

    #[test]
    fn test_normalize_code_handles_escaped_strings() {
        let code = r#"let s = "hello\"world";"#;
        let normalized = CodeUnit::normalize_code(code);
        assert!(normalized.contains("\"$STR\""));
        assert!(!normalized.contains("hello"));
    }

    #[test]
    fn test_normalize_code_handles_floats() {
        let code = "let x = 3.14;";
        let normalized = CodeUnit::normalize_code(code);
        assert!(normalized.contains("$NUM"));
        assert!(!normalized.contains("3.14"));
    }

    #[test]
    fn test_call_hierarchy_item_stable_id() {
        let item = CallHierarchyItem {
            name: "foo".to_string(),
            file_path: "/src/lib.rs".to_string(),
            line: 42,
            column: 4,
        };
        assert_eq!(item.stable_id(), "/src/lib.rs:42:foo");
    }
}
