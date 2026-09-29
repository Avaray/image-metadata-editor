use std::fmt;

/// Top-level error type. It distinguishes usage errors (bad flags or
/// arguments, exit code 2) from runtime errors (bad file, bad data, I/O
/// failure, exit code 1), per `02-cli-interface.md`.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Usage(String),
    Runtime(String),
}

impl Error {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Runtime(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Runtime(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}
