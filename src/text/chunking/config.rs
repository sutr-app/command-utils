//! Configuration and provider traits for hierarchical text chunking

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Generic token provider trait for text tokenization
///
/// This trait abstracts tokenization functionality to allow different
/// tokenizer implementations (llama.cpp, HuggingFace tokenizers, etc.)
pub trait TokenProvider: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Tokenize a single text string into token IDs
    fn tokenize(&self, text: &str) -> Result<Vec<u32>, Self::Error>;

    /// Batch tokenize multiple texts (optional optimization)
    fn tokenize_batch(&self, texts: &[&str]) -> Result<Vec<Vec<u32>>, Self::Error> {
        texts.iter().map(|text| self.tokenize(text)).collect()
    }

    /// Get estimated token count without full tokenization (optional fast path)
    fn estimate_token_count(&self, text: &str) -> Result<usize, Self::Error> {
        self.tokenize(text).map(|tokens| tokens.len())
    }

    /// Convert token position to character position in original text (optional)
    fn token_to_char(&self, _text: &str, _token_pos: usize) -> Result<Option<usize>, Self::Error>;

    /// Convert character position to token position in original text (optional)
    fn char_to_token(&self, _text: &str, _char_pos: usize) -> Result<Option<usize>, Self::Error>;

    /// Get token spans (char start/end for each token) if supported
    fn get_token_spans(&self, _text: &str) -> Result<Option<Vec<(usize, usize)>>, Self::Error>;
}

/// Configuration for hierarchical text chunking
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchicalChunkingConfig {
    /// Maximum tokens per chunk
    pub max_chunk_tokens: usize,
    /// Minimum tokens per chunk (for merging small chunks)
    pub min_chunk_tokens: usize,
    /// Enable merging of small paragraphs
    pub enable_paragraph_merging: bool,
    /// Enable sentence-based splitting for large paragraphs
    pub enable_sentence_splitting: bool,
    /// Enable forced splitting when sentence splitting is insufficient
    pub enable_forced_splitting: bool,
}

impl Default for HierarchicalChunkingConfig {
    fn default() -> Self {
        Self {
            max_chunk_tokens: 1024,
            min_chunk_tokens: 50,
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
        }
    }
}

impl HierarchicalChunkingConfig {
    /// Create configuration optimized for embedding generation
    pub fn for_embedding(max_tokens: usize) -> Self {
        Self {
            max_chunk_tokens: max_tokens,
            min_chunk_tokens: 5 as usize, // very small minimum to allow small chunks
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
        }
    }

    /// Create configuration optimized for speed
    pub fn for_speed() -> Self {
        Self {
            max_chunk_tokens: 512,
            min_chunk_tokens: 20,
            enable_paragraph_merging: false, // Skip merging for speed
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
        }
    }

    /// Create configuration optimized for quality
    pub fn for_quality() -> Self {
        Self {
            max_chunk_tokens: 1536,
            min_chunk_tokens: 100,
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
        }
    }

    /// Validate configuration settings
    pub fn validate(&self) -> Result<(), String> {
        if self.max_chunk_tokens == 0 {
            return Err("max_chunk_tokens must be greater than 0".to_string());
        }

        if self.min_chunk_tokens >= self.max_chunk_tokens {
            return Err("min_chunk_tokens must be less than max_chunk_tokens".to_string());
        }

        if !self.enable_sentence_splitting && !self.enable_forced_splitting {
            return Err("At least one splitting method must be enabled".to_string());
        }


        Ok(())
    }

}

/// Strategy for chunking when no token provider is available
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackStrategy {
    /// Use character count estimation (rough 4 chars per token)
    CharacterEstimation,
    /// Split by character limit only
    CharacterLimit,
    /// Error if no token provider available
    RequireTokenProvider,
}

impl Default for FallbackStrategy {
    fn default() -> Self {
        Self::CharacterEstimation
    }
}

/// Performance and statistical information for chunking operations
#[derive(Debug, Clone, Default)]
pub struct ChunkingStatistics {
    /// Total processing time
    pub total_processing_time: Duration,
    /// Time spent on tokenization
    pub tokenization_time: Duration,
    /// Time spent on paragraph detection
    pub paragraph_detection_time: Duration,
    /// Time spent on sentence splitting
    pub sentence_splitting_time: Duration,
    /// Time spent on forced splitting
    pub forced_splitting_time: Duration,
    /// Time spent on character position adjustment
    pub position_adjustment_time: Duration,

    /// Input text statistics
    pub input_char_count: usize,
    pub input_line_count: usize,
    pub detected_paragraph_count: usize,

    /// Output chunk statistics
    pub total_chunks_created: usize,
    pub complete_paragraph_chunks: usize,
    pub merged_paragraph_chunks: usize,
    pub split_paragraph_chunks: usize,
    pub sentence_based_chunks: usize,
    pub forced_split_chunks: usize,

    /// Token statistics
    pub total_tokens_processed: usize,
    pub avg_tokens_per_chunk: f32,
    pub max_tokens_in_chunk: usize,
    pub min_tokens_in_chunk: usize,

    /// Quality metrics
    pub paragraph_boundary_preservation_rate: f32,
    pub sentence_boundary_preservation_rate: f32,

    /// Performance metrics
    pub chars_per_second: f32,
    pub tokens_per_second: f32,
    pub chunks_per_second: f32,

    /// Memory usage estimation
    pub estimated_peak_memory_mb: f32,

    /// Additional custom metrics
    pub custom_metrics: HashMap<String, f64>,
}

impl ChunkingStatistics {
    /// Create new empty statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Start timing for total processing
    pub fn start_total_timing(&mut self) -> Instant {
        Instant::now()
    }

    /// Finish total timing
    pub fn finish_total_timing(&mut self, start: Instant) {
        self.total_processing_time = start.elapsed();
    }

    /// Record tokenization timing
    pub fn record_tokenization_time(&mut self, duration: Duration) {
        self.tokenization_time += duration;
    }

    /// Record paragraph detection timing
    pub fn record_paragraph_detection_time(&mut self, duration: Duration) {
        self.paragraph_detection_time += duration;
    }

    /// Record sentence splitting timing
    pub fn record_sentence_splitting_time(&mut self, duration: Duration) {
        self.sentence_splitting_time += duration;
    }

    /// Record forced splitting timing
    pub fn record_forced_splitting_time(&mut self, duration: Duration) {
        self.forced_splitting_time += duration;
    }

    /// Record position adjustment timing
    pub fn record_position_adjustment_time(&mut self, duration: Duration) {
        self.position_adjustment_time += duration;
    }

    /// Record input text statistics
    pub fn record_input_stats(&mut self, text: &str) {
        self.input_char_count = text.len();
        self.input_line_count = text.lines().count();
    }

    /// Record chunk creation by type
    pub fn record_chunk_creation(&mut self, chunk_type: &super::types::ChunkType) {
        use super::types::ChunkType;

        self.total_chunks_created += 1;
        match chunk_type {
            ChunkType::CompleteParagraph => self.complete_paragraph_chunks += 1,
            ChunkType::MergedParagraphs => self.merged_paragraph_chunks += 1,
            ChunkType::SplitParagraph => self.split_paragraph_chunks += 1,
            ChunkType::SentenceBasedSplit => self.sentence_based_chunks += 1,
            ChunkType::ForcedSplit => self.forced_split_chunks += 1,
            ChunkType::Custom(_) => {} // Don't count custom types in standard metrics
        }
    }

    /// Record token statistics for a chunk
    pub fn record_token_stats(&mut self, token_count: usize) {
        self.total_tokens_processed += token_count;

        if self.max_tokens_in_chunk == 0 || token_count > self.max_tokens_in_chunk {
            self.max_tokens_in_chunk = token_count;
        }

        if self.min_tokens_in_chunk == 0 || token_count < self.min_tokens_in_chunk {
            self.min_tokens_in_chunk = token_count;
        }
    }

    /// Calculate derived metrics (call this after all processing is complete)
    pub fn calculate_derived_metrics(&mut self) {
        // Average tokens per chunk
        if self.total_chunks_created > 0 {
            self.avg_tokens_per_chunk =
                self.total_tokens_processed as f32 / self.total_chunks_created as f32;
        }

        // Boundary preservation rates
        let boundary_preserving_chunks =
            self.complete_paragraph_chunks + self.merged_paragraph_chunks;
        if self.total_chunks_created > 0 {
            self.paragraph_boundary_preservation_rate =
                boundary_preserving_chunks as f32 / self.total_chunks_created as f32;
        }

        let sentence_preserving_chunks = boundary_preserving_chunks + self.sentence_based_chunks;
        if self.total_chunks_created > 0 {
            self.sentence_boundary_preservation_rate =
                sentence_preserving_chunks as f32 / self.total_chunks_created as f32;
        }

        // Performance metrics
        let total_seconds = self.total_processing_time.as_secs_f32();
        if total_seconds > 0.0 {
            self.chars_per_second = self.input_char_count as f32 / total_seconds;
            self.tokens_per_second = self.total_tokens_processed as f32 / total_seconds;
            self.chunks_per_second = self.total_chunks_created as f32 / total_seconds;
        }

        // Estimate peak memory usage (rough calculation)
        let avg_chunk_size = if self.total_chunks_created > 0 {
            self.input_char_count / self.total_chunks_created
        } else {
            0
        };
        self.estimated_peak_memory_mb =
            (self.input_char_count + avg_chunk_size * self.total_chunks_created * 2) as f32
                / 1_048_576.0;
    }

    /// Add a custom metric
    pub fn add_custom_metric(&mut self, name: String, value: f64) {
        self.custom_metrics.insert(name, value);
    }

    /// Get summary as string for logging
    pub fn summary(&self) -> String {
        format!(
            "Chunking Stats: {} chars → {} chunks ({:.1} avg tokens/chunk) in {:.2}ms | \
            Para preservation: {:.1}%, Sent preservation: {:.1}% | \
            Speed: {:.0} chars/s, {:.0} tokens/s, {:.1} chunks/s",
            self.input_char_count,
            self.total_chunks_created,
            self.avg_tokens_per_chunk,
            self.total_processing_time.as_millis(),
            self.paragraph_boundary_preservation_rate * 100.0,
            self.sentence_boundary_preservation_rate * 100.0,
            self.chars_per_second,
            self.tokens_per_second,
            self.chunks_per_second
        )
    }
}

/// Cache for tokenization results to improve performance
#[derive(Debug, Clone)]
pub struct TokenizationCache {
    /// Cache for text -> token count estimates
    estimation_cache: HashMap<String, usize>,
    /// Cache for text -> full tokenization results
    tokenization_cache: HashMap<String, Vec<u32>>,
    /// Maximum cache size to prevent memory bloat
    max_cache_size: usize,
    /// Enable/disable caching
    enabled: bool,
}

impl Default for TokenizationCache {
    fn default() -> Self {
        Self {
            estimation_cache: HashMap::new(),
            tokenization_cache: HashMap::new(),
            max_cache_size: 1000,
            enabled: true,
        }
    }
}

impl TokenizationCache {
    /// Create new cache with specified max size
    pub fn new(max_size: usize) -> Self {
        Self {
            max_cache_size: max_size,
            ..Default::default()
        }
    }

    /// Create disabled cache (pass-through mode)
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Get cached token count estimate
    pub fn get_estimate(&self, text: &str) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        self.estimation_cache.get(text).copied()
    }

    /// Cache token count estimate
    pub fn cache_estimate(&mut self, text: String, count: usize) {
        if !self.enabled {
            return;
        }

        if self.estimation_cache.len() >= self.max_cache_size {
            // Simple eviction: clear half the cache
            let keys_to_remove: Vec<_> = self
                .estimation_cache
                .keys()
                .take(self.max_cache_size / 2)
                .cloned()
                .collect();
            for key in keys_to_remove {
                self.estimation_cache.remove(&key);
            }
        }

        self.estimation_cache.insert(text, count);
    }

    /// Get cached tokenization result
    pub fn get_tokens(&self, text: &str) -> Option<Vec<u32>> {
        if !self.enabled {
            return None;
        }
        self.tokenization_cache.get(text).cloned()
    }

    /// Cache tokenization result
    pub fn cache_tokens(&mut self, text: String, tokens: Vec<u32>) {
        if !self.enabled {
            return;
        }

        if self.tokenization_cache.len() >= self.max_cache_size {
            // Simple eviction: clear half the cache
            let keys_to_remove: Vec<_> = self
                .tokenization_cache
                .keys()
                .take(self.max_cache_size / 2)
                .cloned()
                .collect();
            for key in keys_to_remove {
                self.tokenization_cache.remove(&key);
            }
        }

        self.tokenization_cache.insert(text, tokens);
    }

    /// Clear all cached data
    pub fn clear(&mut self) {
        self.estimation_cache.clear();
        self.tokenization_cache.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.estimation_cache.len(),
            self.tokenization_cache.len(),
            self.max_cache_size,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock token provider for testing
    struct MockTokenProvider;

    impl TokenProvider for MockTokenProvider {
        type Error = std::io::Error;

        fn tokenize(&self, text: &str) -> Result<Vec<u32>, Self::Error> {
            // Simple mock: 1 token per 4 characters
            let token_count = text.len().div_ceil(4);
            Ok((1..=token_count as u32).collect())
        }

        fn estimate_token_count(&self, text: &str) -> Result<usize, Self::Error> {
            Ok(text.len().div_ceil(4))
        }
        fn token_to_char(
            &self,
            text: &str,
            token_pos: usize,
        ) -> Result<Option<usize>, Self::Error> {
            let char_pos = token_pos.checked_mul(4);
            if let Some(pos) = char_pos {
                if pos <= text.len() {
                    return Ok(Some(pos));
                }
            }
            Ok(None)
        }
        fn char_to_token(&self, text: &str, char_pos: usize) -> Result<Option<usize>, Self::Error> {
            if char_pos > text.len() {
                return Ok(None);
            }
            Ok(Some(char_pos.div_ceil(4)))
        }
        fn get_token_spans(&self, text: &str) -> Result<Option<Vec<(usize, usize)>>, Self::Error> {
            let token_count = text.len().div_ceil(4);
            let spans = (0..token_count)
                .map(|i| {
                    let start = i * 4;
                    let end = ((i + 1) * 4).min(text.len());
                    (start, end)
                })
                .collect();
            Ok(Some(spans))
        }
    }

    #[test]
    fn test_default_config() {
        let config = HierarchicalChunkingConfig::default();
        assert_eq!(config.max_chunk_tokens, 1024);
        assert_eq!(config.min_chunk_tokens, 50);
        assert!(config.enable_paragraph_merging);
        assert!(config.enable_sentence_splitting);
        assert!(config.enable_forced_splitting);
    }

    #[test]
    fn test_config_validation() {
        let mut config = HierarchicalChunkingConfig::default();
        assert!(config.validate().is_ok());

        // Test invalid configurations
        config.max_chunk_tokens = 0;
        assert!(config.validate().is_err());

        config.max_chunk_tokens = 100;
        config.min_chunk_tokens = 200;
        assert!(config.validate().is_err());

        config.min_chunk_tokens = 50;
        config.enable_sentence_splitting = false;
        config.enable_forced_splitting = false;
        assert!(config.validate().is_err());

        config.enable_forced_splitting = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_preset_configs() {
        let embedding_config = HierarchicalChunkingConfig::for_embedding(512);
        assert_eq!(embedding_config.max_chunk_tokens, 512);
        assert_eq!(embedding_config.min_chunk_tokens, 5); // Fixed minimum for small chunks

        let speed_config = HierarchicalChunkingConfig::for_speed();
        assert_eq!(speed_config.max_chunk_tokens, 512);
        assert!(!speed_config.enable_paragraph_merging);

        let quality_config = HierarchicalChunkingConfig::for_quality();
        assert_eq!(quality_config.max_chunk_tokens, 1536);
        assert_eq!(quality_config.min_chunk_tokens, 100);
    }


    #[test]
    fn test_mock_token_provider() {
        let provider = MockTokenProvider;

        // Test simple tokenization
        let tokens = provider.tokenize("hello world test").unwrap();
        assert_eq!(tokens.len(), 4); // 16 chars / 4 = 4 tokens

        let estimated = provider.estimate_token_count("hello world test").unwrap();
        assert_eq!(estimated, 4);

        // Test batch tokenization
        let texts = vec!["short", "longer text here"];
        let batch_results = provider.tokenize_batch(&texts).unwrap();
        assert_eq!(batch_results.len(), 2);
        assert_eq!(batch_results[0].len(), 2); // "short" = 5 chars = 2 tokens
        assert_eq!(batch_results[1].len(), 4); // "longer text here" = 16 chars = 4 tokens
    }

    #[test]
    fn test_fallback_strategy() {
        assert_eq!(
            FallbackStrategy::default(),
            FallbackStrategy::CharacterEstimation
        );

        let strategies = [
            FallbackStrategy::CharacterEstimation,
            FallbackStrategy::CharacterLimit,
            FallbackStrategy::RequireTokenProvider,
        ];

        for strategy in strategies {
            // Just test that they can be created and compared
            assert_eq!(strategy, strategy);
        }
    }

    #[test]
    fn test_chunking_statistics() {
        use crate::text::chunking::types::ChunkType;
        use std::time::Duration;

        let mut stats = ChunkingStatistics::new();

        // Test input stats recording
        let text = "Sample text for testing";
        stats.record_input_stats(text);
        assert_eq!(stats.input_char_count, text.len());
        assert_eq!(stats.input_line_count, 1);

        // Test chunk creation recording
        stats.record_chunk_creation(&ChunkType::CompleteParagraph);
        stats.record_chunk_creation(&ChunkType::SentenceBasedSplit);
        stats.record_token_stats(50);
        stats.record_token_stats(75);

        // Test timing recording
        stats.record_tokenization_time(Duration::from_millis(10));
        stats.record_paragraph_detection_time(Duration::from_millis(5));

        // Calculate derived metrics
        stats.calculate_derived_metrics();

        assert_eq!(stats.total_chunks_created, 2);
        assert_eq!(stats.complete_paragraph_chunks, 1);
        assert_eq!(stats.sentence_based_chunks, 1);
        assert_eq!(stats.total_tokens_processed, 125);
        assert_eq!(stats.avg_tokens_per_chunk, 62.5);
        assert_eq!(stats.max_tokens_in_chunk, 75);
        assert_eq!(stats.min_tokens_in_chunk, 50);
        assert_eq!(stats.paragraph_boundary_preservation_rate, 0.5); // 1/2

        // Test custom metrics
        stats.add_custom_metric("test_metric".to_string(), 123.45);
        assert_eq!(stats.custom_metrics.get("test_metric"), Some(&123.45));

        // Test summary
        let summary = stats.summary();
        assert!(summary.contains("2")); // total chunks
        assert!(summary.contains("62.5")); // avg tokens per chunk
    }

    #[test]
    fn test_tokenization_cache() {
        let mut cache = TokenizationCache::new(3); // Small cache for testing

        // Test estimate caching
        assert!(cache.get_estimate("test").is_none());
        cache.cache_estimate("test".to_string(), 5);
        assert_eq!(cache.get_estimate("test"), Some(5));

        // Test tokenization caching
        assert!(cache.get_tokens("test").is_none());
        cache.cache_tokens("test".to_string(), vec![1, 2, 3]);
        assert_eq!(cache.get_tokens("test"), Some(vec![1, 2, 3]));

        // Test cache eviction
        cache.cache_estimate("test2".to_string(), 10);
        cache.cache_estimate("test3".to_string(), 15);
        cache.cache_estimate("test4".to_string(), 20); // Should trigger eviction

        let (est_size, token_size, max_size) = cache.stats();
        assert!(est_size <= max_size);
        assert!(token_size <= max_size);
        assert_eq!(max_size, 3);

        // Test cache clearing
        cache.clear();
        let (est_size, token_size, _) = cache.stats();
        assert_eq!(est_size, 0);
        assert_eq!(token_size, 0);
    }

    #[test]
    fn test_disabled_cache() {
        let mut cache = TokenizationCache::disabled();

        cache.cache_estimate("test".to_string(), 5);
        cache.cache_tokens("test".to_string(), vec![1, 2, 3]);

        assert!(cache.get_estimate("test").is_none());
        assert!(cache.get_tokens("test").is_none());

        let (est_size, token_size, _) = cache.stats();
        assert_eq!(est_size, 0);
        assert_eq!(token_size, 0);
    }
}
