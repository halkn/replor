use std::path::PathBuf;

/// Fatal error categories. Every variant maps to exit code 1; usage errors are
/// reported by clap itself and exit with 2.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("no input: stdin is a terminal and no FILE was given")]
    MissingInput,

    #[error("{path}: is a directory, not a Markdown file")]
    InputIsDirectory { path: PathBuf },

    #[error("{path}: {source}")]
    InputIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("stdin: {source}")]
    StdinIo {
        #[source]
        source: std::io::Error,
    },

    #[error("{origin}: input is not valid UTF-8 at byte {offset}")]
    Decode { origin: String, offset: usize },

    #[error("terminal: {source}")]
    Terminal {
        #[source]
        source: std::io::Error,
    },

    #[error("{path}: cannot watch for changes: {message}")]
    Watch { path: PathBuf, message: String },

    #[error("output: {source}")]
    Output {
        #[source]
        source: std::io::Error,
    },
}

impl AppError {
    pub const EXIT_FAILURE: i32 = 1;

    /// A closed downstream pipe is how `| head` and `fzf --preview` normally
    /// end, so it is a successful exit rather than a failure.
    pub fn is_broken_pipe(&self) -> bool {
        matches!(
            self,
            AppError::Output { source } if source.kind() == std::io::ErrorKind::BrokenPipe
        )
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn io(kind: std::io::ErrorKind) -> AppError {
        AppError::Output {
            source: std::io::Error::new(kind, "test"),
        }
    }

    #[test]
    fn only_a_broken_output_pipe_is_treated_as_success() {
        assert!(io(std::io::ErrorKind::BrokenPipe).is_broken_pipe());
        assert!(!io(std::io::ErrorKind::PermissionDenied).is_broken_pipe());
        assert!(!AppError::MissingInput.is_broken_pipe());
        assert!(
            !AppError::Terminal {
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test"),
            }
            .is_broken_pipe()
        );
    }
}
