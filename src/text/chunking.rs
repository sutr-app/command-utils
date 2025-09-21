//! Hierarchical text chunking for RAG-optimized embedding generation
//!
//! This module provides paragraph-aware hierarchical text chunking that prioritizes
//! semantic boundaries for better embedding quality in RAG applications.

pub mod chunker;
pub mod config;
pub mod error;
pub mod sliding_window;
pub mod types;

// Re-export main public interfaces
pub use chunker::HierarchicalChunker;
pub use config::{
    ChunkingStatistics, FallbackStrategy, HierarchicalChunkingConfig, TokenProvider,
    TokenizationCache,
};
pub use error::{HierarchicalChunkingError, Result};
pub use sliding_window::{EmbeddingMerger, MergeStrategy, SlidingWindowCalculator};
pub use types::{ChunkType, HierarchicalChunk};
