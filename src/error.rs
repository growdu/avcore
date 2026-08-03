//! 统一错误类型与退出码

use std::process::ExitCode;

/// AVCore 统一错误
#[derive(thiserror::Error, Debug)]
pub enum AvcError {
    #[error("{0}")]
    Generic(String),

    #[error("参数错：{0}")]
    Arg(String),

    #[error("资源不存在：{0}")]
    NotFound(String),

    #[error("状态冲突：{0}")]
    Conflict(String),

    #[error("token 未配置：{0}")]
    TokenMissing(String),

    #[error("token 鉴权失败：{0}")]
    TokenAuth(String),

    #[error("Provider 限速：{0}")]
    RateLimited(String),

    #[error("Provider 上游错：{0}")]
    ProviderUpstream(String),

    #[error("Provider 超时：{0}")]
    ProviderTimeout(String),

    #[error("NL 模型未配置：{0}")]
    NlModelMissing(String),

    #[error("数据库：{0}")]
    Db(String),

    #[error("IO：{0}")]
    Io(String),

    #[error("内部错：{0}")]
    Internal(String),

    #[error("daemon already running: pid {pid}, port {port}")]
    AlreadyRunning { pid: u32, port: u16 },

    #[error("daemon bind failed on {addr}:{port}: {msg}")]
    BindFailed {
        addr: String,
        port: u16,
        msg: String,
    },

    #[error("pidfile stale: {0}")]
    PidfileStale(String),

    #[error("daemon not running")]
    DaemonNotRunning,
}

impl AvcError {
    pub fn code(&self) -> ExitCode {
        match self {
            AvcError::Arg(_) => ExitCode::from(2),
            AvcError::NotFound(_) => ExitCode::from(3),
            AvcError::Conflict(_) => ExitCode::from(4),
            AvcError::TokenAuth(_) => ExitCode::from(5),
            AvcError::TokenMissing(_) => ExitCode::from(6),
            AvcError::NlModelMissing(_) => ExitCode::from(6),
            AvcError::RateLimited(_) => ExitCode::from(10),
            AvcError::ProviderUpstream(_) => ExitCode::from(11),
            AvcError::ProviderTimeout(_) => ExitCode::from(12),
            AvcError::Generic(_) => ExitCode::from(1),
            AvcError::Db(_) => ExitCode::from(20),
            AvcError::Io(_) => ExitCode::from(21),
            AvcError::AlreadyRunning { .. } => ExitCode::from(4),
            AvcError::BindFailed { .. } => ExitCode::from(21),
            AvcError::PidfileStale(_) => ExitCode::from(4),
            AvcError::DaemonNotRunning => ExitCode::from(3),
            AvcError::Internal(_) => ExitCode::from(99),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        self.code()
    }

    pub fn print(&self) {
        // 简化输出，正式错误格式见 docs/cli.md §5.6
        eprintln!("error: {}", self);
    }
}

impl From<std::io::Error> for AvcError {
    fn from(e: std::io::Error) -> Self {
        AvcError::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for AvcError {
    fn from(e: rusqlite::Error) -> Self {
        AvcError::Db(e.to_string())
    }
}

impl From<serde_json::Error> for AvcError {
    fn from(e: serde_json::Error) -> Self {
        AvcError::Generic(format!("json: {}", e))
    }
}

impl From<toml::de::Error> for AvcError {
    fn from(e: toml::de::Error) -> Self {
        AvcError::Generic(format!("toml decode: {}", e))
    }
}

impl From<toml::ser::Error> for AvcError {
    fn from(e: toml::ser::Error) -> Self {
        AvcError::Generic(format!("toml encode: {}", e))
    }
}

pub type AvcResult<T> = Result<T, AvcError>;
