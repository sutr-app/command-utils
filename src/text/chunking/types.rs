//! Core data structures for hierarchical text chunking

use std::collections::HashMap;

/// Hierarchical chunk data structure for text processing
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchicalChunk {
    /// Chunk content
    pub content: String,
    /// Token IDs (provided by TokenProvider)
    pub tokens: Vec<u32>,
    /// Character start position in original text
    pub char_start: usize,
    /// Character end position in original text
    pub char_end: usize,
    /// Type of chunking applied
    pub chunk_type: ChunkType,
    /// Index of this chunk in the sequence
    pub chunk_index: usize,
    /// Extended metadata for customization
    pub metadata: HashMap<String, String>,
}

impl HierarchicalChunk {
    /// Create a new hierarchical chunk
    pub fn new(
        content: String,
        tokens: Vec<u32>,
        char_start: usize,
        char_end: usize,
        chunk_type: ChunkType,
        chunk_index: usize,
    ) -> Self {
        Self {
            content,
            tokens,
            char_start,
            char_end,
            chunk_type,
            chunk_index,
            metadata: HashMap::new(),
        }
    }

    /// Get the length of the chunk in characters
    pub fn char_length(&self) -> usize {
        self.char_end - self.char_start
    }

    /// Get the number of tokens in this chunk
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    /// Check if this chunk is empty
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Get character position range as a tuple
    pub fn char_range(&self) -> (usize, usize) {
        (self.char_start, self.char_end)
    }

    /// Add metadata to this chunk
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }

    /// Get metadata value by key
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}

/// Types of chunking strategies applied to create chunks
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChunkType {
    /// Complete paragraph kept intact
    CompleteParagraph,
    /// Multiple small paragraphs merged together
    MergedParagraphs,
    /// Large paragraph split into smaller pieces
    SplitParagraph,
    /// Sentence-based splitting applied
    SentenceBasedSplit,
    /// Forced splitting by character/token limit
    ForcedSplit,
    /// Custom splitting strategy (extensible)
    Custom(String),
}

impl ChunkType {
    /// Check if this chunk type represents a natural boundary preservation
    pub fn preserves_boundaries(&self) -> bool {
        matches!(
            self,
            ChunkType::CompleteParagraph | ChunkType::MergedParagraphs
        )
    }

    /// Check if this chunk type required forced splitting
    pub fn is_forced_split(&self) -> bool {
        matches!(self, ChunkType::ForcedSplit)
    }

    /// Get a human-readable description of the chunk type
    pub fn description(&self) -> &'static str {
        match self {
            ChunkType::CompleteParagraph => "Complete paragraph",
            ChunkType::MergedParagraphs => "Merged small paragraphs",
            ChunkType::SplitParagraph => "Split large paragraph",
            ChunkType::SentenceBasedSplit => "Sentence-based split",
            ChunkType::ForcedSplit => "Forced character/token split",
            ChunkType::Custom(_) => "Custom splitting strategy",
        }
    }
}

impl std::fmt::Display for ChunkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChunkType::Custom(name) => write!(f, "Custom({name})"),
            _ => write!(f, "{}", self.description()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hierarchical_chunk_creation() {
        let chunk = HierarchicalChunk::new(
            "これはテストです。".to_string(),
            vec![1, 2, 3, 4, 5],
            0,
            10,
            ChunkType::CompleteParagraph,
            0,
        );

        assert_eq!(chunk.content, "これはテストです。");
        assert_eq!(chunk.tokens, vec![1, 2, 3, 4, 5]);
        assert_eq!(chunk.char_start, 0);
        assert_eq!(chunk.char_end, 10);
        assert_eq!(chunk.chunk_type, ChunkType::CompleteParagraph);
        assert_eq!(chunk.chunk_index, 0);
        assert_eq!(chunk.char_length(), 10);
        assert_eq!(chunk.token_count(), 5);
        assert!(!chunk.is_empty());
        assert_eq!(chunk.char_range(), (0, 10));
    }

    #[test]
    fn test_chunk_metadata() {
        let mut chunk = HierarchicalChunk::new(
            "test".to_string(),
            vec![1],
            0,
            4,
            ChunkType::CompleteParagraph,
            0,
        );

        chunk.add_metadata("quality".to_string(), "high".to_string());
        assert_eq!(chunk.get_metadata("quality"), Some(&"high".to_string()));
        assert_eq!(chunk.get_metadata("missing"), None);
    }

    #[test]
    fn test_chunk_type_properties() {
        assert!(ChunkType::CompleteParagraph.preserves_boundaries());
        assert!(ChunkType::MergedParagraphs.preserves_boundaries());
        assert!(!ChunkType::SplitParagraph.preserves_boundaries());
        assert!(!ChunkType::SentenceBasedSplit.preserves_boundaries());
        assert!(!ChunkType::ForcedSplit.preserves_boundaries());

        assert!(!ChunkType::CompleteParagraph.is_forced_split());
        assert!(ChunkType::ForcedSplit.is_forced_split());

        assert_eq!(
            ChunkType::CompleteParagraph.description(),
            "Complete paragraph"
        );
        assert_eq!(
            ChunkType::ForcedSplit.description(),
            "Forced character/token split"
        );
    }

    #[test]
    fn test_chunk_type_display() {
        assert_eq!(
            format!("{}", ChunkType::CompleteParagraph),
            "Complete paragraph"
        );
        assert_eq!(
            format!("{}", ChunkType::Custom("special".to_string())),
            "Custom(special)"
        );
    }

    #[test]
    fn test_empty_chunk() {
        let chunk = HierarchicalChunk::new("".to_string(), vec![], 0, 0, ChunkType::ForcedSplit, 0);

        assert!(chunk.is_empty());
        assert_eq!(chunk.char_length(), 0);
        assert_eq!(chunk.token_count(), 0);
    }
}
