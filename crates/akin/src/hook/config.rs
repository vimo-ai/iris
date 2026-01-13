//! Hook 配置

/// Hook 配置
#[derive(Debug, Clone)]
pub struct HookConfig {
    pub threshold: f32,
    pub min_lines: u32,
    pub scope: HookScope,
    pub max_results: usize,
    pub notify: NotifyMode,
    pub model: String,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            threshold: 0.85,
            min_lines: 5,
            scope: HookScope::All,
            max_results: 3,
            notify: NotifyMode::Block,
            model: "bge-m3".to_string(),
        }
    }
}

impl HookConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(v) = std::env::var("AKIN_THRESHOLD") {
            if let Ok(t) = v.parse() {
                config.threshold = t;
            }
        }

        if let Ok(v) = std::env::var("AKIN_MIN_LINES") {
            if let Ok(m) = v.parse() {
                config.min_lines = m;
            }
        }

        if let Ok(v) = std::env::var("AKIN_SCOPE") {
            config.scope = match v.as_str() {
                "project" => HookScope::Project,
                "cross" => HookScope::CrossOnly,
                _ => HookScope::All,
            };
        }

        if let Ok(v) = std::env::var("AKIN_MAX_RESULTS") {
            if let Ok(m) = v.parse() {
                config.max_results = m;
            }
        }

        if let Ok(v) = std::env::var("AKIN_NOTIFY") {
            config.notify = match v.as_str() {
                "user" => NotifyMode::User,
                _ => NotifyMode::Block,
            };
        }

        if let Ok(v) = std::env::var("AKIN_MODEL") {
            config.model = v;
        }

        config
    }
}

/// 检查范围
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookScope {
    /// 检查所有已索引的代码
    All,
    /// 仅检查当前项目
    Project,
    /// 仅检查跨项目的相似
    CrossOnly,
}

/// 通知模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyMode {
    /// 阻止操作
    Block,
    /// 仅通知用户
    User,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_config_default() {
        let config = HookConfig::default();
        assert_eq!(config.threshold, 0.85);
        assert_eq!(config.min_lines, 5);
        assert_eq!(config.max_results, 3);
    }
}
