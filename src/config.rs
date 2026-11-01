use serde::Deserialize;
use tracing;

#[derive(Clone, Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_addr")]
    pub web_addr: String,

    #[serde(default = "default_port")]
    pub web_port: u16,

    #[serde(default = "default_templates_path")]
    pub templates_path: String,

    #[serde(default = "default_assets_path")]
    pub assets_path: String,

    #[serde(default = "default_database_path")]
    pub database_path: String,

    #[serde(default = "default_admin_path")]
    pub admin_path: String,
}

fn default_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_templates_path() -> String {
    "templates".to_string()
}

fn default_assets_path() -> String {
    "assets".to_string()
}

fn default_database_path() -> String {
    "data/data.db".to_string()
}

fn default_admin_path() -> String {
    "admin".to_string()
}

const RESERVED_PATHS: [&str; 3] = ["blog", "thinking", "about"];

fn is_usable_path(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !RESERVED_PATHS.contains(&segment)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            web_addr: default_addr(),
            web_port: default_port(),
            templates_path: default_templates_path(),
            assets_path: default_assets_path(),
            database_path: default_database_path(),
            admin_path: default_admin_path(),
        }
    }
}

impl Config {
    pub fn init() -> Self {
        dotenvy::dotenv().ok();

        let mut config = envy::from_env::<Config>().unwrap_or_else(|err| {
            tracing::warn!("配置信息读取错误: {}, 使用默认配置", err);
            Self::default()
        });
        config.fix_admin_path();
        tracing::info!("配置初始化成功!\n{:#?}", config);
        config
    }

    fn fix_admin_path(&mut self) {
        let trimmed = self.admin_path.trim().trim_matches('/').trim();
        if !is_usable_path(trimmed) {
            tracing::warn!("ADMIN_PATH {:?} 不可用, 回落到默认路径", self.admin_path);
            self.admin_path = default_admin_path();
        } else {
            self.admin_path = trimmed.to_string();
        }
    }

    pub fn admin_prefix(&self) -> String {
        format!("/{}", self.admin_path)
    }
}
