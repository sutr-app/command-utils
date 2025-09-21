//! Error types for hierarchical text chunking

/// Error types for hierarchical chunking operations
#[derive(thiserror::Error, Debug)]
pub enum HierarchicalChunkingError {
    #[error("Text parsing failed: {0}")]
    TextParsing(String),

    #[error("Paragraph boundary detection failed: {0}")]
    ParagraphDetection(String),

    #[error("Sentence splitting failed: {0}")]
    SentenceSplitting(#[from] anyhow::Error),

    #[error("Token provider error: {0}")]
    TokenProvider(String),

    #[error("Tokenization failed: {0}")]
    Tokenization(String),

    #[error("Chunk size validation failed: expected <= {max}, got {actual}")]
    ChunkSizeValidation { max: usize, actual: usize },

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Regex compilation error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Character encoding error: {0}")]
    Encoding(String),

    #[error("Internal processing error: {0}")]
    Internal(String),
}

/// Result type for command-utils chunking operations
pub type Result<T> = std::result::Result<T, HierarchicalChunkingError>;

/// Trait for converting external errors into chunking errors
pub trait IntoChunkingError<T> {
    fn into_chunking_error(self) -> Result<T>;
}

impl<T, E> IntoChunkingError<T> for std::result::Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn into_chunking_error(self) -> Result<T> {
        self.map_err(|e| HierarchicalChunkingError::TokenProvider(e.to_string()))
    }
}

impl HierarchicalChunkingError {
    /// Create a text parsing error
    pub fn text_parsing<S: Into<String>>(msg: S) -> Self {
        Self::TextParsing(msg.into())
    }

    /// Create a paragraph detection error
    pub fn paragraph_detection<S: Into<String>>(msg: S) -> Self {
        Self::ParagraphDetection(msg.into())
    }

    /// Create a token provider error
    pub fn token_provider<S: Into<String>>(msg: S) -> Self {
        Self::TokenProvider(msg.into())
    }

    /// Create a tokenization error
    pub fn tokenization<S: Into<String>>(msg: S) -> Self {
        Self::Tokenization(msg.into())
    }

    /// Create a chunk size validation error
    pub fn chunk_size_validation(max: usize, actual: usize) -> Self {
        Self::ChunkSizeValidation { max, actual }
    }

    /// Create a configuration error
    pub fn configuration<S: Into<String>>(msg: S) -> Self {
        Self::Configuration(msg.into())
    }

    /// Create a validation error
    pub fn validation<S: Into<String>>(msg: S) -> Self {
        Self::Validation(msg.into())
    }

    /// Create an encoding error
    pub fn encoding<S: Into<String>>(msg: S) -> Self {
        Self::Encoding(msg.into())
    }

    /// Create an internal processing error
    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Configuration and validation errors are typically not recoverable
            Self::Configuration(_) | Self::Validation(_) => false,
            // Token provider and I/O errors might be temporary
            Self::TokenProvider(_) | Self::Io(_) => true,
            // Text processing errors might be recoverable with different input
            Self::TextParsing(_)
            | Self::ParagraphDetection(_)
            | Self::SentenceSplitting(_)
            | Self::Tokenization(_)
            | Self::Encoding(_) => true,
            // Size validation might be recoverable with different limits
            Self::ChunkSizeValidation { .. } => true,
            // Regex errors are typically not recoverable
            Self::Regex(_) => false,
            // Internal errors are usually not recoverable
            Self::Internal(_) => false,
        }
    }

    /// Get error category for logging/monitoring
    pub fn category(&self) -> &'static str {
        match self {
            Self::TextParsing(_) => "text_parsing",
            Self::ParagraphDetection(_) => "paragraph_detection",
            Self::SentenceSplitting(_) => "sentence_splitting",
            Self::TokenProvider(_) => "token_provider",
            Self::Tokenization(_) => "tokenization",
            Self::ChunkSizeValidation { .. } => "chunk_size_validation",
            Self::Configuration(_) => "configuration",
            Self::Validation(_) => "validation",
            Self::Io(_) => "io",
            Self::Regex(_) => "regex",
            Self::Encoding(_) => "encoding",
            Self::Internal(_) => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let error = HierarchicalChunkingError::text_parsing("failed to parse");
        assert!(matches!(error, HierarchicalChunkingError::TextParsing(_)));
        assert_eq!(error.to_string(), "Text parsing failed: failed to parse");

        let error = HierarchicalChunkingError::chunk_size_validation(100, 200);
        assert!(matches!(
            error,
            HierarchicalChunkingError::ChunkSizeValidation { .. }
        ));
        assert_eq!(
            error.to_string(),
            "Chunk size validation failed: expected <= 100, got 200"
        );
    }

    #[test]
    fn test_error_categories() {
        let error = HierarchicalChunkingError::text_parsing("test");
        assert_eq!(error.category(), "text_parsing");

        let error = HierarchicalChunkingError::configuration("test");
        assert_eq!(error.category(), "configuration");

        let error = HierarchicalChunkingError::chunk_size_validation(10, 20);
        assert_eq!(error.category(), "chunk_size_validation");
    }

    #[test]
    fn test_error_recoverability() {
        // Non-recoverable errors
        assert!(!HierarchicalChunkingError::configuration("test").is_recoverable());
        assert!(!HierarchicalChunkingError::validation("test").is_recoverable());
        assert!(!HierarchicalChunkingError::internal("test").is_recoverable());

        // Potentially recoverable errors
        assert!(HierarchicalChunkingError::text_parsing("test").is_recoverable());
        assert!(HierarchicalChunkingError::token_provider("test").is_recoverable());
        assert!(HierarchicalChunkingError::chunk_size_validation(10, 20).is_recoverable());
    }

    #[test]
    fn test_into_chunking_error() {
        let io_error: std::io::Result<()> = Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));

        let chunking_result = io_error.into_chunking_error();
        assert!(chunking_result.is_err());
        assert!(matches!(
            chunking_result.unwrap_err(),
            HierarchicalChunkingError::TokenProvider(_)
        ));
    }

    #[test]
    fn test_error_from_conversions() {
        // Test From<std::io::Error>
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let chunking_error: HierarchicalChunkingError = io_error.into();
        assert!(matches!(chunking_error, HierarchicalChunkingError::Io(_)));

        // Test From<anyhow::Error>
        let anyhow_error = anyhow::anyhow!("test error");
        let chunking_error: HierarchicalChunkingError = anyhow_error.into();
        assert!(matches!(
            chunking_error,
            HierarchicalChunkingError::SentenceSplitting(_)
        ));
    }
}
