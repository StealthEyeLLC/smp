use std::io;

#[derive(Debug, thiserror::Error)]
pub enum SmpError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("ambiguous state: {0}")]
    Ambiguous(String),
    #[error("state error: {0}")]
    State(String),
    #[error("external operation failed: {program} exited {code}: {stderr}")]
    External {
        program: String,
        code: i32,
        stderr: String,
    },
    #[error("guest command exited {0}")]
    GuestExit(i32),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("JSON error at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, SmpError>;

impl SmpError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Invalid(_) => 2,
            Self::NotFound(_) => 3,
            Self::Conflict(_) => 4,
            Self::Ambiguous(_) => 5,
            Self::State(_) => 6,
            Self::External { code, .. } | Self::GuestExit(code) => {
                u8::try_from((*code).clamp(1, 255)).unwrap_or(1)
            }
            Self::Io { .. } | Self::Json { .. } => 10,
        }
    }

    pub fn io(path: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}
