//! 配置加载：`~/.config/avc/avc.toml`
//!
//! 与 SQLite 分离：备份库不泄漏密钥。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AvcError, AvcResult};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: Providers,
    #[serde(default)]
    pub shell: ShellCfg,
    #[serde(default)]
    pub safety: SafetyCfg,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Providers {
    #[serde(default)]
    pub avatar: HashMap<String, ProviderCfg>,
    #[serde(default)]
    pub voice: HashMap<String, ProviderCfg>,
    #[serde(default)]
    pub llm: HashMap<String, ProviderCfg>,
    #[serde(default)]
    pub video: HashMap<String, ProviderCfg>,
    #[serde(default)]
    pub embed: HashMap<String, ProviderCfg>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderCfg {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCfg {
    pub nl_model: Option<String>,
    pub max_plan_steps: Option<u32>,
    pub temperature: Option<f32>,
}

impl Default for ShellCfg {
    fn default() -> Self {
        Self {
            nl_model: None,
            max_plan_steps: Some(8),
            temperature: Some(0.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCfg {
    pub real_person_enabled: Option<bool>,
    pub auto_consume_feedback: Option<bool>,
}

impl Default for SafetyCfg {
    fn default() -> Self {
        Self {
            real_person_enabled: Some(false),
            auto_consume_feedback: Some(true),
        }
    }
}

impl Config {
    pub fn default_config_path() -> AvcResult<PathBuf> {
        let base = dirs::config_dir()
            .ok_or_else(|| crate::error::AvcError::Generic("无法定位 config_dir".into()))?;
        Ok(base.join("avc").join("avc.toml"))
    }

    pub fn default_data_dir() -> AvcResult<PathBuf> {
        let base = dirs::data_dir()
            .ok_or_else(|| crate::error::AvcError::Generic("无法定位 data_dir".into()))?;
        Ok(base.join("avc"))
    }

    pub fn default_db_path() -> AvcResult<PathBuf> {
        Ok(Self::default_data_dir()?.join("avc.db"))
    }

    /// 加载 avc.toml；若不存在返回默认
    pub fn load(path: &Path) -> AvcResult<Self> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> AvcResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        // 0600 权限（仅 Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }
        Ok(())
    }

    /// 只读点号路径查询（与 `apply_set` 范围一致）：
    /// 仅支持 `provider.<dim>.<name>.{api_key|model|endpoint}`。
    /// 未知维度/字段/形状返回 `AvcError::Arg`；已知路径但未设置返回 `None`。
    pub fn get_path(&self, key: &str) -> AvcResult<Option<String>> {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() != 4 || parts[0] != "provider" {
            return Err(AvcError::Arg(format!("暂不支持此 key: {}", key)));
        }
        let dim = parts[1];
        let name = parts[2];
        let field = parts[3];
        if dim.is_empty() || name.is_empty() || field.is_empty() {
            return Err(AvcError::Arg(format!("暂不支持此 key: {}", key)));
        }

        let map = match dim {
            "avatar" => &self.provider.avatar,
            "voice" => &self.provider.voice,
            "llm" => &self.provider.llm,
            "video" => &self.provider.video,
            "embed" => &self.provider.embed,
            _ => return Err(AvcError::Arg(format!("未知维度: {}", dim))),
        };

        let entry = match map.get(name) {
            Some(e) => e,
            None => return Ok(None),
        };
        let val = match field {
            "api_key" => entry.api_key.as_ref(),
            "model" => entry.model.as_ref(),
            "endpoint" => entry.endpoint.as_ref(),
            _ => return Err(AvcError::Arg(format!("未知字段: {}", field))),
        };
        Ok(val.map(|s| s.clone()))
    }
}
