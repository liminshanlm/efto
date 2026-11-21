use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("插件清单无效: {0}")]
    InvalidManifest(String),

    #[error("索引未就绪")]
    NotIndexed,

    #[error("启动失败: {0}")]
    Launch(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io {
            path: PathBuf::from("<unknown>"),
            source: e,
        }
    }
}
