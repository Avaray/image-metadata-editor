use std::fmt;

pub enum AppError {
    Usage(String),
    Runtime(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Usage(msg) => write!(f, "Usage error: {}", msg),
            AppError::Runtime(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::Usage(_) => 2,
            AppError::Runtime(_) => 1,
        }
    }
}
