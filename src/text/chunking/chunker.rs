//! Main hierarchical chunker implementation with paragraph-aware splitting

use super::{
    config::{
        ChunkingStatistics, FallbackStrategy, HierarchicalChunkingConfig, TokenProvider,
        TokenizationCache,
    },
    error::{HierarchicalChunkingError, Result},
    types::{ChunkType, HierarchicalChunk},
};
use crate::text::{SentenceSplitter, SentenceSplitterCreator};
use regex::Regex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Information about a detected paragraph including its position
#[derive(Debug, Clone)]
struct ParagraphInfo {
    content: String,
    char_start: usize,
    char_end: usize,
}

/// Hierarchical text chunker for RAG-optimized embedding generation
pub struct HierarchicalChunker<T: TokenProvider> {
    config: HierarchicalChunkingConfig,
    token_provider: Option<Arc<T>>,
    sentence_splitter: SentenceSplitter,
    fallback_strategy: FallbackStrategy,
    paragraph_regex: Regex,
    /// Performance and statistics tracking
    statistics: ChunkingStatistics,
    /// Tokenization cache for performance optimization
    tokenization_cache: TokenizationCache,
}

impl<T: TokenProvider> HierarchicalChunker<T> {
    /// Create a new hierarchical chunker with token provider
    pub fn new(
        config: HierarchicalChunkingConfig,
        token_provider: T,
        fallback_strategy: Option<FallbackStrategy>,
    ) -> Result<Self> {
        config
            .validate()
            .map_err(HierarchicalChunkingError::configuration)?;

        let sentence_splitter = SentenceSplitterCreator::new(None, None, None, None)
            .create()
            .map_err(|e| {
                HierarchicalChunkingError::configuration(format!(
                    "Failed to create sentence splitter: {e}"
                ))
            })?;

        // Compile paragraph boundary detection regex
        let paragraph_regex = Regex::new(r"\n\s*\n|\n\s*[　\t]")?;

        Ok(Self {
            config,
            token_provider: Some(Arc::new(token_provider)),
            sentence_splitter,
            fallback_strategy: fallback_strategy.unwrap_or_default(),
            paragraph_regex,
            statistics: ChunkingStatistics::new(),
            tokenization_cache: TokenizationCache::default(),
        })
    }

    /// Create a new hierarchical chunker without token provider (fallback mode)
    pub fn new_fallback(
        config: HierarchicalChunkingConfig,
        fallback_strategy: FallbackStrategy,
    ) -> Result<Self> {
        if fallback_strategy == FallbackStrategy::RequireTokenProvider {
            return Err(HierarchicalChunkingError::configuration(
                "Token provider required but not provided".to_string(),
            ));
        }

        config
            .validate()
            .map_err(HierarchicalChunkingError::configuration)?;

        let sentence_splitter = SentenceSplitterCreator::new(None, None, None, None)
            .create()
            .map_err(|e| {
                HierarchicalChunkingError::configuration(format!(
                    "Failed to create sentence splitter: {e}"
                ))
            })?;

        let paragraph_regex = Regex::new(r"\n\s*\n|\n\s*[　\t]")?;

        Ok(Self {
            config,
            token_provider: None,
            sentence_splitter,
            fallback_strategy,
            paragraph_regex,
            statistics: ChunkingStatistics::new(),
            tokenization_cache: TokenizationCache::default(),
        })
    }

    /// Main chunking method: hierarchical paragraph-aware text splitting
    pub fn chunk_efficiently(&mut self, text: &str) -> Result<Vec<HierarchicalChunk>> {
        debug!(
            "Starting hierarchical chunking for text of {} characters",
            text.len()
        );

        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Initialize statistics tracking
        let total_start = self.statistics.start_total_timing();
        self.statistics.record_input_stats(text);

        // Step 1: Fast paragraph boundary detection
        let para_start = Instant::now();
        let paragraphs = self.detect_paragraph_boundaries_fast(text)?;
        self.statistics
            .record_paragraph_detection_time(para_start.elapsed());
        self.statistics.detected_paragraph_count = paragraphs.len();
        info!("Detected {} paragraphs", paragraphs.len());

        // Step 2: Process each paragraph through 3-level hierarchy
        let mut final_chunks = Vec::new();
        let mut pending_small_paragraphs = Vec::new();

        for (paragraph_index, paragraph_info) in paragraphs.into_iter().enumerate() {
            let paragraph_trimmed = paragraph_info.content.trim();
            if paragraph_trimmed.is_empty() {
                continue;
            }

            let token_count = self.calculate_token_count(paragraph_trimmed)?;

            if token_count <= self.config.max_chunk_tokens {
                if token_count >= self.config.min_chunk_tokens {
                    // Perfect size - use as complete paragraph
                    let tokens = self.tokenize_text(paragraph_trimmed)?;
                    let chunk = self.create_chunk(
                        paragraph_trimmed.to_string(),
                        tokens,
                        paragraph_info.char_start,
                        paragraph_info.char_end,
                        ChunkType::CompleteParagraph,
                        final_chunks.len(),
                    );
                    final_chunks.push(chunk);
                    debug!(
                        "Added complete paragraph chunk: {} tokens, position: {}..{}",
                        token_count, paragraph_info.char_start, paragraph_info.char_end
                    );
                } else {
                    // Too small - consider for merging
                    let tokens = self.tokenize_text(paragraph_trimmed)?;
                    pending_small_paragraphs.push((
                        paragraph_trimmed.to_string(),
                        tokens,
                        paragraph_index,
                    ));
                    debug!(
                        "Paragraph too small ({} tokens), pending merge",
                        token_count
                    );
                }
            } else {
                // Too large - apply sentence-based splitting
                debug!(
                    "Paragraph too large ({} tokens), applying sentence splitting",
                    token_count
                );
                // For now, use the original method and adjust positions later
                // TODO: Implement position-aware sentence splitting
                let split_chunks = self.split_paragraph_by_sentences(paragraph_trimmed)?;
                for chunk in split_chunks {
                    final_chunks.push(self.adjust_chunk_index(chunk, final_chunks.len()));
                }
            }
        }

        // Step 3: Process small paragraphs (merge if enabled)
        if self.config.enable_paragraph_merging && !pending_small_paragraphs.is_empty() {
            debug!(
                "Processing {} small paragraphs for merging",
                pending_small_paragraphs.len()
            );
            let merged_chunks = self.merge_small_paragraphs_simple(pending_small_paragraphs)?;
            for chunk in merged_chunks {
                final_chunks.push(self.adjust_chunk_index(chunk, final_chunks.len()));
            }
        } else {
            // Add small paragraphs individually if merging is disabled
            for (content, tokens, _) in pending_small_paragraphs {
                let chunk = self.create_chunk(
                    content.clone(),
                    tokens,
                    0,
                    content.len(),
                    ChunkType::CompleteParagraph,
                    final_chunks.len(),
                );
                final_chunks.push(chunk);
            }
        }

        // Step 4: Adjust character positions based on original text
        let pos_start = Instant::now();
        self.adjust_character_positions(text, &mut final_chunks)?;
        self.statistics
            .record_position_adjustment_time(pos_start.elapsed());

        // Step 5: Filter out zero-length chunks
        let pre_filter_count = final_chunks.len();
        final_chunks.retain(|chunk| {
            let length = chunk.char_end.saturating_sub(chunk.char_start);
            if length == 0 {
                warn!(
                    "Filtering out zero-length chunk: char_start={}, char_end={}, content_len={}",
                    chunk.char_start,
                    chunk.char_end,
                    chunk.content.len()
                );
                false
            } else {
                true
            }
        });

        if final_chunks.len() != pre_filter_count {
            info!(
                "Filtered {} zero-length chunks, {} chunks remaining",
                pre_filter_count - final_chunks.len(),
                final_chunks.len()
            );
        }

        // Step 5.5: Filter out chunks that don't meet minimum token requirement
        let pre_min_filter_count = final_chunks.len();
        final_chunks.retain(|chunk| {
            let token_count = chunk.tokens.len();
            if token_count < self.config.min_chunk_tokens {
                warn!(
                    "Filtering out chunk with {} tokens (< min_chunk_tokens={}): {:?}",
                    token_count,
                    self.config.min_chunk_tokens,
                    chunk.content.chars().take(50).collect::<String>()
                );
                false
            } else {
                true
            }
        });

        if final_chunks.len() != pre_min_filter_count {
            info!(
                "Filtered {} chunks below min_chunk_tokens={}, {} chunks remaining",
                pre_min_filter_count - final_chunks.len(),
                self.config.min_chunk_tokens,
                final_chunks.len()
            );
        }

        // Step 6: Sort chunks by position to ensure proper ordering
        final_chunks.sort_by_key(|chunk| (chunk.char_start, chunk.char_end));
        debug!("Sorted {} chunks by character position", final_chunks.len());

        // Update chunk indices after sorting
        for (idx, chunk) in final_chunks.iter_mut().enumerate() {
            chunk.chunk_index = idx;
        }

        // Finalize statistics
        self.statistics.finish_total_timing(total_start);
        self.statistics.calculate_derived_metrics();

        info!(
            "Hierarchical chunking completed: {} final chunks",
            final_chunks.len()
        );
        debug!("{}", self.statistics.summary());

        Ok(final_chunks)
    }

    /// Fast paragraph boundary detection using regex patterns
    fn detect_paragraph_boundaries_fast(&self, text: &str) -> Result<Vec<ParagraphInfo>> {
        debug!("Detecting paragraph boundaries with regex");

        let mut paragraphs = Vec::new();
        let mut last_end = 0;

        for mat in self.paragraph_regex.find_iter(text) {
            let start = mat.start();
            if start > last_end {
                let paragraph = &text[last_end..start];
                if !paragraph.trim().is_empty() {
                    paragraphs.push(ParagraphInfo {
                        content: paragraph.to_string(),
                        char_start: last_end,
                        char_end: start,
                    });
                }
            }
            last_end = mat.end();
        }

        // Add the remaining text as the last paragraph
        if last_end < text.len() {
            let remaining = &text[last_end..];
            if !remaining.trim().is_empty() {
                paragraphs.push(ParagraphInfo {
                    content: remaining.to_string(),
                    char_start: last_end,
                    char_end: text.len(),
                });
            }
        }

        // If no paragraphs were found, treat the entire text as one paragraph
        if paragraphs.is_empty() && !text.trim().is_empty() {
            paragraphs.push(ParagraphInfo {
                content: text.trim().to_string(),
                char_start: 0,
                char_end: text.len(),
            });
        }

        debug!("Detected {} paragraph boundaries", paragraphs.len());
        Ok(paragraphs)
    }

    /// Split large paragraph by sentences (Level 2 processing)
    fn split_paragraph_by_sentences(&mut self, paragraph: &str) -> Result<Vec<HierarchicalChunk>> {
        debug!(
            "Splitting paragraph by sentences: {} chars",
            paragraph.len()
        );

        let sent_start = Instant::now();

        if !self.config.enable_sentence_splitting {
            // If sentence splitting is disabled, apply forced splitting
            return self.apply_forced_splitting(paragraph);
        }

        let sentences = self.sentence_splitter.split(paragraph.to_string());
        let mut chunks = Vec::new();
        let mut current_sentences = Vec::new();
        let mut current_char_pos = 0;

        for sentence in sentences {
            let sentence_trimmed = sentence.trim();
            if sentence_trimmed.is_empty() {
                continue;
            }

            // Calculate combined text and token count
            let combined_text = if current_sentences.is_empty() {
                sentence_trimmed.to_string()
            } else {
                format!("{} {}", current_sentences.join(" "), sentence_trimmed)
            };

            let token_count = self.calculate_token_count(&combined_text)?;

            if token_count <= self.config.max_chunk_tokens {
                current_sentences.push(sentence_trimmed.to_string());
            } else {
                // Current group is full, finalize it
                if !current_sentences.is_empty() {
                    let content = current_sentences.join(" ");
                    let tokens = self.tokenize_text(&content)?;
                    let chunk = self.create_chunk(
                        content.clone(),
                        tokens,
                        current_char_pos,
                        current_char_pos + content.len(),
                        ChunkType::SentenceBasedSplit,
                        chunks.len(),
                    );
                    chunks.push(chunk);
                    current_char_pos += content.len();
                }

                // Start new group with current sentence
                current_sentences = vec![sentence_trimmed.to_string()];

                // Check if single sentence is too large
                let single_token_count = self.calculate_token_count(sentence_trimmed)?;
                if single_token_count > self.config.max_chunk_tokens {
                    warn!("Single sentence exceeds token limit, applying forced splitting");
                    let forced_chunks = self.apply_forced_splitting(sentence_trimmed)?;
                    for chunk in forced_chunks {
                        let chunk_len = chunk.content.len();
                        chunks.push(self.adjust_chunk_char_positions(chunk, current_char_pos));
                        current_char_pos += chunk_len;
                    }
                    current_sentences.clear();
                }
            }
        }

        // Process remaining sentences
        if !current_sentences.is_empty() {
            let content = current_sentences.join(" ");
            let token_count = self.calculate_token_count(&content)?;

            if token_count <= self.config.max_chunk_tokens {
                let tokens = self.tokenize_text(&content)?;
                let chunk = self.create_chunk(
                    content.clone(),
                    tokens,
                    current_char_pos,
                    current_char_pos + content.len(),
                    ChunkType::SentenceBasedSplit,
                    chunks.len(),
                );
                chunks.push(chunk);
            } else {
                // Content exceeds token limit, apply forced splitting
                warn!(
                    "Remaining sentences exceed token limit ({}), applying forced splitting",
                    token_count
                );
                let forced_chunks = self.apply_forced_splitting(&content)?;
                for chunk in forced_chunks {
                    let chunk_len = chunk.content.len();
                    chunks.push(self.adjust_chunk_char_positions(chunk, current_char_pos));
                    current_char_pos += chunk_len;
                }
            }
        }

        self.statistics
            .record_sentence_splitting_time(sent_start.elapsed());
        debug!(
            "Split paragraph into {} sentence-based chunks",
            chunks.len()
        );
        Ok(chunks)
    }

    /// Apply forced splitting when other methods fail (Level 3 processing)
    fn apply_forced_splitting(&mut self, text: &str) -> Result<Vec<HierarchicalChunk>> {
        if !self.config.enable_forced_splitting {
            return Err(HierarchicalChunkingError::configuration(
                "Forced splitting is disabled but required".to_string(),
            ));
        }

        debug!("Applying forced splitting to text of {} chars", text.len());
        let forced_start = Instant::now();

        let mut chunks = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut start_pos = 0;

        while start_pos < chars.len() {
            let end_pos = self.find_forced_split_position(&chars, start_pos)?;

            let chunk_text: String = chars[start_pos..end_pos].iter().collect();
            let tokens = self.tokenize_text(&chunk_text)?;

            let chunk = self.create_chunk(
                chunk_text,
                tokens,
                start_pos,
                end_pos,
                ChunkType::ForcedSplit,
                chunks.len(),
            );
            chunks.push(chunk);

            start_pos = end_pos;
        }

        self.statistics
            .record_forced_splitting_time(forced_start.elapsed());
        debug!("Created {} forced split chunks", chunks.len());
        Ok(chunks)
    }

    /// Find optimal position for forced splitting based on token count
    fn find_forced_split_position(&mut self, chars: &[char], start_pos: usize) -> Result<usize> {
        let remaining_chars = chars.len() - start_pos;

        if remaining_chars == 0 {
            return Ok(chars.len());
        }

        // Start with a conservative estimate based on token limit
        let initial_estimate = std::cmp::min(
            start_pos + (self.config.max_chunk_tokens * 3), // Conservative 3 chars per token
            chars.len(),
        );

        // Binary search to find the maximum position that fits within token limit
        let mut low = start_pos + 1;
        let mut high = initial_estimate;
        let mut best_pos = low;

        while low <= high {
            let mid = (low + high) / 2;
            let test_text: String = chars[start_pos..mid].iter().collect();

            match self.calculate_token_count(&test_text) {
                Ok(token_count) if token_count <= self.config.max_chunk_tokens => {
                    best_pos = mid;
                    low = mid + 1;
                }
                Ok(_) => {
                    high = mid - 1;
                }
                Err(_) => {
                    // If tokenization fails, fall back to character estimation
                    high = mid - 1;
                }
            }
        }

        // Try to find a good breaking point near the optimal position
        let search_range = 20; // Search within 20 characters
        let search_start = best_pos.saturating_sub(search_range);
        let search_end = std::cmp::min(best_pos + search_range, chars.len());

        // Look for good break characters in reverse order from best position
        for i in (search_start..std::cmp::min(best_pos, search_end)).rev() {
            if chars[i].is_whitespace() || "。！？.!?、,".contains(chars[i]) {
                let test_text: String = chars[start_pos..i + 1].iter().collect();
                if let Ok(token_count) = self.calculate_token_count(&test_text) {
                    if token_count <= self.config.max_chunk_tokens {
                        return Ok(i + 1);
                    }
                }
            }
        }

        // If no good break point found, use the best position we found
        Ok(best_pos)
    }

    /// Merge small paragraphs (Level 3 processing)
    fn merge_small_paragraphs_simple(
        &mut self,
        small_paragraphs: Vec<(String, Vec<u32>, usize)>,
    ) -> Result<Vec<HierarchicalChunk>> {
        debug!("Merging {} small paragraphs", small_paragraphs.len());

        let mut merged_chunks = Vec::new();
        let mut current_group = Vec::new();
        let mut _current_tokens = 0;

        for (paragraph, paragraph_tokens, _) in small_paragraphs {
            let combined_text = if current_group.is_empty() {
                paragraph.clone()
            } else {
                format!("{}\n\n{}", current_group.join("\n\n"), paragraph)
            };

            let combined_token_count = self.calculate_token_count(&combined_text)?;

            if combined_token_count <= self.config.max_chunk_tokens {
                // Add to current group
                current_group.push(paragraph);
                _current_tokens = combined_token_count;
            } else {
                // Finalize current group
                if !current_group.is_empty() {
                    let content = current_group.join("\n\n");
                    let tokens = self.tokenize_text(&content)?;
                    let chunk = self.create_chunk(
                        content,
                        tokens,
                        0, // Will be adjusted later
                        0, // Will be adjusted later
                        ChunkType::MergedParagraphs,
                        merged_chunks.len(),
                    );
                    merged_chunks.push(chunk);
                }

                // Start new group
                current_group = vec![paragraph];
                _current_tokens = paragraph_tokens.len();
            }
        }

        // Process remaining group
        if !current_group.is_empty() {
            let content = current_group.join("\n\n");
            let tokens = self.tokenize_text(&content)?;
            let chunk = self.create_chunk(
                content,
                tokens,
                0, // Will be adjusted later
                0, // Will be adjusted later
                ChunkType::MergedParagraphs,
                merged_chunks.len(),
            );
            merged_chunks.push(chunk);
        }

        debug!(
            "Created {} merged chunks from small paragraphs",
            merged_chunks.len()
        );
        Ok(merged_chunks)
    }

    /// Calculate token count for text
    fn calculate_token_count(&mut self, text: &str) -> Result<usize> {
        let token_start = Instant::now();

        // Check cache first
        if let Some(cached_count) = self.tokenization_cache.get_estimate(text) {
            return Ok(cached_count);
        }

        let result = if let Some(provider) = &self.token_provider {
            provider
                .estimate_token_count(text)
                .map_err(|e| HierarchicalChunkingError::token_provider(e.to_string()))
        } else {
            match self.fallback_strategy {
                FallbackStrategy::CharacterEstimation => {
                    // Rough estimate: 4 characters per token
                    Ok(text.len().div_ceil(4))
                }
                FallbackStrategy::CharacterLimit => {
                    // Use character count directly
                    Ok(text.len())
                }
                FallbackStrategy::RequireTokenProvider => {
                    Err(HierarchicalChunkingError::configuration(
                        "Token provider required but not available".to_string(),
                    ))
                }
            }
        };

        // Cache the result if successful
        if let Ok(count) = &result {
            self.tokenization_cache
                .cache_estimate(text.to_string(), *count);
        }

        self.statistics
            .record_tokenization_time(token_start.elapsed());
        result
    }

    /// Tokenize text using provider or fallback
    fn tokenize_text(&mut self, text: &str) -> Result<Vec<u32>> {
        let token_start = Instant::now();

        // Check cache first
        if let Some(cached_tokens) = self.tokenization_cache.get_tokens(text) {
            return Ok(cached_tokens);
        }

        let result = if let Some(provider) = &self.token_provider {
            provider
                .tokenize(text)
                .map_err(|e| HierarchicalChunkingError::tokenization(e.to_string()))
        } else {
            // Fallback: generate dummy tokens based on character estimation
            let estimated_count = self.calculate_token_count(text)?;
            Ok((1..=estimated_count as u32).collect())
        };

        // Cache the result if successful
        if let Ok(tokens) = &result {
            self.tokenization_cache
                .cache_tokens(text.to_string(), tokens.clone());
        }

        self.statistics
            .record_tokenization_time(token_start.elapsed());
        result
    }

    /// Create a hierarchical chunk
    fn create_chunk(
        &mut self,
        content: String,
        tokens: Vec<u32>,
        char_start: usize,
        char_end: usize,
        chunk_type: ChunkType,
        chunk_index: usize,
    ) -> HierarchicalChunk {
        // Record statistics
        self.statistics.record_chunk_creation(&chunk_type);
        self.statistics.record_token_stats(tokens.len());

        HierarchicalChunk::new(
            content,
            tokens,
            char_start,
            char_end,
            chunk_type,
            chunk_index,
        )
    }

    /// Adjust chunk index
    fn adjust_chunk_index(
        &self,
        mut chunk: HierarchicalChunk,
        new_index: usize,
    ) -> HierarchicalChunk {
        chunk.chunk_index = new_index;
        chunk
    }

    /// Adjust chunk character positions
    fn adjust_chunk_char_positions(
        &self,
        mut chunk: HierarchicalChunk,
        offset: usize,
    ) -> HierarchicalChunk {
        chunk.char_start += offset;
        chunk.char_end += offset;
        chunk
    }

    /// Adjust character positions for all chunks based on original text
    fn adjust_character_positions(
        &mut self,
        original_text: &str,
        chunks: &mut [HierarchicalChunk],
    ) -> Result<()> {
        debug!("Adjusting character positions for {} chunks", chunks.len());
        debug!("Original text length: {} chars", original_text.len());

        // Try to use tokenizer-based position calculation if available
        if let Some(provider) = &self.token_provider {
            debug!("Token provider available, attempting tokenizer-based positioning");
            match provider.get_token_spans(original_text) {
                Ok(Some(token_spans)) => {
                    debug!("Got {} token spans from tokenizer", token_spans.len());
                    self.adjust_positions_with_tokenizer(original_text, chunks, &token_spans)?;
                    return Ok(());
                }
                Ok(None) => {
                    debug!("Tokenizer returned None for token spans");
                }
                Err(e) => {
                    debug!("Failed to get token spans: {:?}", e);
                }
            }
        } else {
            debug!("No token provider available");
        }

        // Fallback to string-based approach
        debug!("Using string-based position calculation fallback");
        self.adjust_positions_with_string_search(original_text, chunks)?;

        Ok(())
    }

    /// Use tokenizer to calculate precise character positions
    fn adjust_positions_with_tokenizer(
        &mut self,
        original_text: &str,
        chunks: &mut [HierarchicalChunk],
        token_spans: &[(usize, usize)],
    ) -> Result<()> {
        debug!("Starting tokenizer-based position adjustment");
        debug!("Token spans available: {}", token_spans.len());

        let text_len = original_text.chars().count();
        let mut current_token_pos = 0;

        for (chunk_idx, chunk) in chunks.iter_mut().enumerate() {
            debug!(
                "Processing chunk {}: {} tokens, current_token_pos={}",
                chunk_idx,
                chunk.tokens.len(),
                current_token_pos
            );

            // Skip if positions are already correctly set and the first chunk doesn't start at 0
            // OR if this is not the first chunk and char_start is properly non-zero
            let positions_look_correct =
                (chunk_idx == 0 && chunk.char_start == 0 && chunk.char_end > 0)
                    || (chunk_idx > 0 && chunk.char_start > 0 && chunk.char_end > chunk.char_start);

            if positions_look_correct {
                debug!(
                    "Chunk {} has correct positions: {}..{}",
                    chunk_idx, chunk.char_start, chunk.char_end
                );
                current_token_pos += chunk.tokens.len(); // Still need to advance position

                // Apply safety check even for "correct" positions
                if chunk.char_end > text_len {
                    warn!(
                        "Chunk {} end position {} exceeds text length {}, clamping",
                        chunk_idx, chunk.char_end, text_len
                    );
                    chunk.char_end = text_len;
                }
                if chunk.char_start > text_len {
                    warn!(
                        "Chunk {} start position {} exceeds text length {}, clamping",
                        chunk_idx, chunk.char_start, text_len
                    );
                    chunk.char_start = text_len;
                }
                if chunk.char_start > chunk.char_end {
                    warn!(
                        "Chunk {} start position {} exceeds end position {}, clamping start",
                        chunk_idx, chunk.char_start, chunk.char_end
                    );
                    chunk.char_start = chunk.char_end;
                }
                continue;
            }

            let chunk_token_count = chunk.tokens.len();
            if current_token_pos + chunk_token_count <= token_spans.len() {
                // Get character positions from token spans
                chunk.char_start = token_spans[current_token_pos].0;
                chunk.char_end = token_spans[current_token_pos + chunk_token_count - 1].1;

                // Safety check: ensure positions don't exceed text length
                if chunk.char_end > text_len {
                    warn!(
                        "Chunk {} end position {} exceeds text length {}, clamping",
                        chunk_idx, chunk.char_end, text_len
                    );
                    chunk.char_end = text_len;
                }

                debug!(
                    "Chunk {} positioned: {}..{} (tokens {}..{})",
                    chunk_idx,
                    chunk.char_start,
                    chunk.char_end,
                    current_token_pos,
                    current_token_pos + chunk_token_count - 1
                );
                current_token_pos += chunk_token_count;
            } else {
                warn!("Not enough token spans for chunk positioning: need {} tokens, have {} remaining",
                    chunk_token_count, token_spans.len() - current_token_pos);
                // Fallback to estimated positioning
                let estimated_start = if current_token_pos < token_spans.len() {
                    token_spans[current_token_pos].0
                } else {
                    text_len
                };
                chunk.char_start = estimated_start;
                chunk.char_end = (estimated_start + chunk.content.chars().count()).min(text_len);
            }
        }

        Ok(())
    }

    /// Fallback string-based position calculation
    fn adjust_positions_with_string_search(
        &mut self,
        original_text: &str,
        chunks: &mut [HierarchicalChunk],
    ) -> Result<()> {
        let mut current_search_pos = 0;

        for (chunk_idx, chunk) in chunks.iter_mut().enumerate() {
            debug!(
                "Processing chunk {} for string-based positioning",
                chunk_idx
            );
            debug!("  Content: {:?}", chunk.content);
            debug!(
                "  Current positions: {}-{}",
                chunk.char_start, chunk.char_end
            );

            // Always recalculate positions to ensure accuracy

            // Try to find the chunk content in the original text
            if let Some(pos) = original_text[current_search_pos..].find(&chunk.content) {
                let actual_start = current_search_pos + pos;
                chunk.char_start = actual_start;
                chunk.char_end = actual_start + chunk.content.len();
                current_search_pos = chunk.char_end;
            } else {
                // Fallback: try without the leading search position constraint
                if let Some(pos) = original_text.find(&chunk.content) {
                    chunk.char_start = pos;
                    chunk.char_end = pos + chunk.content.len();
                } else {
                    warn!(
                        "Could not find chunk content in original text: {}",
                        &chunk.content[..50.min(chunk.content.len())]
                    );
                    // Fallback to sequential positioning
                    chunk.char_start = current_search_pos;
                    chunk.char_end = current_search_pos + chunk.content.len();
                    current_search_pos = chunk.char_end;
                }
            }

            // Safety check
            let text_len = original_text.len();
            if chunk.char_end > text_len {
                chunk.char_end = text_len;
                if chunk.char_start > text_len {
                    chunk.char_start = text_len;
                }
            }
        }

        Ok(())
    }

    /// Get configuration reference
    pub fn config(&self) -> &HierarchicalChunkingConfig {
        &self.config
    }

    /// Check if token provider is available
    pub fn has_token_provider(&self) -> bool {
        self.token_provider.is_some()
    }

    /// Get statistics for the last chunking operation
    pub fn statistics(&self) -> &ChunkingStatistics {
        &self.statistics
    }

    /// Reset statistics for a new chunking operation
    pub fn reset_statistics(&mut self) {
        self.statistics = ChunkingStatistics::new();
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (usize, usize, usize) {
        self.tokenization_cache.stats()
    }

    /// Clear tokenization cache
    pub fn clear_cache(&mut self) {
        self.tokenization_cache.clear();
    }

    /// Configure tokenization cache
    pub fn configure_cache(&mut self, max_size: usize, enabled: bool) {
        if enabled {
            self.tokenization_cache = TokenizationCache::new(max_size);
        } else {
            self.tokenization_cache = TokenizationCache::disabled();
        }
    }

    /// Batch process multiple texts with shared cache
    pub fn batch_chunk_efficiently(
        &mut self,
        texts: &[&str],
    ) -> Result<Vec<Vec<HierarchicalChunk>>> {
        let batch_start = Instant::now();
        let mut results = Vec::with_capacity(texts.len());
        let mut total_input_chars = 0;
        let mut total_output_chunks = 0;

        for text in texts {
            total_input_chars += text.len();
            let chunks = self.chunk_efficiently(text)?;
            total_output_chunks += chunks.len();
            results.push(chunks);
        }

        // Add batch processing metrics
        let batch_time = batch_start.elapsed();
        self.statistics.add_custom_metric(
            "batch_processing_time_ms".to_string(),
            batch_time.as_millis() as f64,
        );
        self.statistics.add_custom_metric(
            "batch_total_input_chars".to_string(),
            total_input_chars as f64,
        );
        self.statistics.add_custom_metric(
            "batch_total_output_chunks".to_string(),
            total_output_chunks as f64,
        );
        self.statistics.add_custom_metric(
            "batch_throughput_chars_per_sec".to_string(),
            total_input_chars as f64 / batch_time.as_secs_f64(),
        );

        info!(
            "Batch processing completed: {} texts, {} total chars, {} total chunks in {:.2}ms",
            texts.len(),
            total_input_chars,
            total_output_chunks,
            batch_time.as_millis()
        );

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    // Mock token provider for testing
    struct MockTokenProvider;

    impl TokenProvider for MockTokenProvider {
        type Error = std::io::Error;

        fn tokenize(&self, text: &str) -> std::result::Result<Vec<u32>, Self::Error> {
            // Simple mock: 1 token per 4 characters
            let token_count = text.len().div_ceil(4);
            Ok((1..=token_count as u32).collect())
        }

        fn estimate_token_count(&self, text: &str) -> std::result::Result<usize, Self::Error> {
            Ok(text.len().div_ceil(4))
        }

        /// Convert token position to character position in original text (optional)
        fn token_to_char(
            &self,
            _text: &str,
            _token_pos: usize,
        ) -> Result<Option<usize>, Self::Error> {
            Ok(None) // Not implemented in mock
        }

        /// Convert character position to token position in original text (optional)
        fn char_to_token(
            &self,
            _text: &str,
            _char_pos: usize,
        ) -> Result<Option<usize>, Self::Error> {
            Ok(None) // Not implemented in mock
        }

        /// Get token spans (char start/end for each token) if supported
        fn get_token_spans(&self, _text: &str) -> Result<Option<Vec<(usize, usize)>>, Self::Error> {
            Ok(None) // Not implemented in mock
        }
    }

    #[test]
    fn test_chunker_creation() {
        let config = HierarchicalChunkingConfig::default();
        let token_provider = MockTokenProvider;

        let chunker = HierarchicalChunker::new(config, token_provider, None);
        assert!(chunker.is_ok());

        let chunker = chunker.unwrap();
        assert!(chunker.has_token_provider());
    }

    #[test]
    fn test_chunker_fallback_mode() {
        let config = HierarchicalChunkingConfig::default();

        let chunker = HierarchicalChunker::<MockTokenProvider>::new_fallback(
            config,
            FallbackStrategy::CharacterEstimation,
        );
        assert!(chunker.is_ok());

        let chunker = chunker.unwrap();
        assert!(!chunker.has_token_provider());
    }

    #[test]
    fn test_paragraph_boundary_detection() {
        let config = HierarchicalChunkingConfig::default();
        let token_provider = MockTokenProvider;
        let chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let text = "第一段落です。これは最初の段落。\n\n第二段落です。これは二番目の段落。\n\n第三段落です。";
        let paragraphs = chunker.detect_paragraph_boundaries_fast(text).unwrap();

        assert_eq!(paragraphs.len(), 3);
        assert!(paragraphs[0].content.contains("第一段落"));
        assert!(paragraphs[1].content.contains("第二段落"));
        assert!(paragraphs[2].content.contains("第三段落"));
    }

    #[test]
    fn test_simple_chunking() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 100,
            min_chunk_tokens: 5, // Lower threshold to treat our test text as complete paragraph
            enable_paragraph_merging: true,
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let text = "これは短いテストです。";
        let chunks = chunker.chunk_efficiently(text).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::CompleteParagraph);
        assert_eq!(chunks[0].content, "これは短いテストです。");
    }

    #[test]
    fn test_forced_splitting() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 10,
            min_chunk_tokens: 5, // Set appropriate min value
            max_char_length_fallback: Some(20),
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Create a long text that will require forced splitting
        let text = "これは非常に長いテキストです。" // This should exceed 10 tokens in our mock
            .repeat(3);

        let chunks = chunker.chunk_efficiently(&text).unwrap();

        // Processing flow:
        // 1. Paragraph detection: 1 paragraph (entire text)
        // 2. Sentence splitting: 3 sentences "これは非常に長いテキストです。" each
        // 3. Each sentence has ~13 tokens, exceeds max_chunk_tokens=10
        // 4. Forced splitting: Each sentence split at "。" boundary
        // Expected: 6 chunks - each sentence split into 2 parts
        // "これは非常に長いテキスト" (~10 tokens) and "です。" (~3 tokens)
        // But "です。" has < 5 tokens (min_chunk_tokens), so should be filtered out
        // Final: 3 chunks of "これは非常に長いテキスト"
        assert_eq!(
            chunks.len(),
            3,
            "Expected exactly 3 chunks after filtering min_chunk_tokens"
        );

        // Verify all chunks respect token limits and minimum
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.tokens.len() <= 10,
                "Chunk {} has {} tokens, expected <= 10",
                i,
                chunk.tokens.len()
            );
            assert!(
                chunk.tokens.len() >= 5,
                "Chunk {} has {} tokens, expected >= 5 (min_chunk_tokens)",
                i,
                chunk.tokens.len()
            );
        }

        // Verify all chunks have the expected content type (forced split)
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.chunk_type,
                ChunkType::ForcedSplit,
                "Chunk {} should be ForcedSplit, got {:?}",
                i,
                chunk.chunk_type
            );
        }

        // Verify exact content matches - should be the main part of each sentence
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.content, "これは非常に長いテキストで",
                "Chunk {} content should be the main part without 'す。'",
                i
            );
        }
    }

    #[test]
    fn test_token_calculation() {
        let config = HierarchicalChunkingConfig::default();
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let text = "test"; // 4 characters = 1 token in our mock
        let token_count = chunker.calculate_token_count(text).unwrap();
        assert_eq!(token_count, 1);

        let longer_text = "this is a longer test"; // 21 characters = 6 tokens
        let token_count = chunker.calculate_token_count(longer_text).unwrap();
        assert_eq!(token_count, 6);
    }

    #[test]
    fn test_fallback_strategy() {
        let config = HierarchicalChunkingConfig::default();
        let mut chunker = HierarchicalChunker::<MockTokenProvider>::new_fallback(
            config,
            FallbackStrategy::CharacterEstimation,
        )
        .unwrap();

        let text = "test"; // 4 characters
        let token_count = chunker.calculate_token_count(text).unwrap();
        assert_eq!(token_count, 1); // 4 chars / 4 = 1 token
    }

    #[test]
    fn test_statistics_collection() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 100,
            min_chunk_tokens: 1, // Very low threshold to keep small chunks for this test
            enable_paragraph_merging: false, // Disable merging for this test
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let text = "First paragraph.\n\nSecond paragraph with more content.";
        let chunks = chunker.chunk_efficiently(text).unwrap();

        let stats = chunker.statistics();
        assert_eq!(chunks.len(), 2); // Should have 2 chunks now
                                     // Processing time might be 0 for fast operations on small text
        assert!(stats.total_processing_time.as_nanos() > 0);
        assert_eq!(stats.input_char_count, text.len());
        assert!(stats.detected_paragraph_count > 0);
        assert!(stats.total_chunks_created > 0);
        assert!(stats.total_tokens_processed > 0);
        // Chars per second might be 0 if processing is very fast
        assert!(stats.chars_per_second >= 0.0);
    }

    #[test]
    fn test_cache_functionality() {
        let config = HierarchicalChunkingConfig::default();
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let text = "test text for caching";

        // First call should populate cache
        let count1 = chunker.calculate_token_count(text).unwrap();
        let tokens1 = chunker.tokenize_text(text).unwrap();

        // Second call should use cache
        let count2 = chunker.calculate_token_count(text).unwrap();
        let tokens2 = chunker.tokenize_text(text).unwrap();

        assert_eq!(count1, count2);
        assert_eq!(tokens1, tokens2);

        let (est_cache_size, token_cache_size, _max_size) = chunker.cache_stats();
        assert!(est_cache_size > 0);
        assert!(token_cache_size > 0);
    }

    #[test]
    fn test_batch_processing() {
        let config = HierarchicalChunkingConfig {
            min_chunk_tokens: 1, // Very low threshold to keep small chunks for this test
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        let texts = vec![
            "First document content.",
            "Second document with more text.",
            "Third document text.",
        ];

        let batch_results = chunker.batch_chunk_efficiently(&texts).unwrap();
        assert_eq!(batch_results.len(), 3);

        for (i, chunks) in batch_results.iter().enumerate() {
            assert!(!chunks.is_empty());
            assert_eq!(chunks[0].content.trim(), texts[i]);
        }

        let stats = chunker.statistics();
        assert!(stats
            .custom_metrics
            .contains_key("batch_processing_time_ms"));
        assert!(stats.custom_metrics.contains_key("batch_total_input_chars"));
        assert!(stats
            .custom_metrics
            .contains_key("batch_total_output_chunks"));
    }

    #[test]
    fn test_cache_configuration() {
        let config = HierarchicalChunkingConfig::default();
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Test disabling cache
        chunker.configure_cache(500, false);
        let text = "test text";
        let _count = chunker.calculate_token_count(text).unwrap();
        let (est_cache_size, token_cache_size, _max_size) = chunker.cache_stats();
        assert_eq!(est_cache_size, 0);
        assert_eq!(token_cache_size, 0);

        // Test enabling cache with different size
        chunker.configure_cache(100, true);
        let _count = chunker.calculate_token_count(text).unwrap();
        let _tokens = chunker.tokenize_text(text).unwrap(); // Add tokenization to populate cache
        let (est_cache_size, token_cache_size, max_size) = chunker.cache_stats();
        assert!(est_cache_size > 0);
        assert!(token_cache_size > 0);
        assert_eq!(max_size, 100);
    }

    #[test]
    fn test_paragraph_level_chunking() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 50, // Large enough to fit each paragraph
            min_chunk_tokens: 5,
            enable_paragraph_merging: false,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Create text with multiple paragraphs of appropriate sizes (each ~19-20 tokens)
        let text = "This is first paragraph.\n\n\
                   This is second paragraph.\n\n\
                   This is third paragraph.";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // Should have 3 chunks (one per paragraph)
        assert_eq!(chunks.len(), 3);

        // Verify chunk types and content
        for chunk in &chunks {
            assert_eq!(chunk.chunk_type, ChunkType::CompleteParagraph);
            assert!(!chunk.content.contains("\n\n")); // No paragraph breaks within chunks
        }

        // Verify statistics
        let stats = chunker.statistics();
        assert_eq!(stats.detected_paragraph_count, 3);
        assert_eq!(stats.complete_paragraph_chunks, 3);
        assert_eq!(stats.sentence_based_chunks, 0);
        assert_eq!(stats.forced_split_chunks, 0);
    }

    #[test]
    fn test_sentence_level_chunking() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 10, // Very small to force splitting
            min_chunk_tokens: 2,
            enable_paragraph_merging: false,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Create text with very clear sentence boundaries
        let text = "First sentence! Second sentence? Third sentence.";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // Should have multiple chunks due to sentence splitting
        assert!(
            chunks.len() > 1,
            "Expected multiple chunks, got {}",
            chunks.len()
        );

        // Should have sentence-based splits since paragraph is too large
        let sentence_chunks = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::SentenceBasedSplit)
            .count();
        assert!(
            sentence_chunks > 0,
            "Expected sentence-based chunks, got {:?}",
            chunks.iter().map(|c| &c.chunk_type).collect::<Vec<_>>()
        );

        // Verify no chunk exceeds token limit
        for chunk in &chunks {
            assert!(
                chunk.token_count() <= 10,
                "Chunk exceeds token limit: {} tokens",
                chunk.token_count()
            );
        }

        // Verify statistics
        let stats = chunker.statistics();
        assert_eq!(stats.detected_paragraph_count, 1);
        assert_eq!(stats.complete_paragraph_chunks, 0); // Paragraph was too large
        assert!(stats.sentence_based_chunks > 0);
    }

    #[test]
    fn test_forced_splitting_level() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 8, // Very small to force character-level splitting
            min_chunk_tokens: 2,
            enable_paragraph_merging: false,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            max_char_length_fallback: Some(30), // Small character limit
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Create text with very long sentences that can't be split naturally
        let text = "ThisIsAnExtremelyLongSentenceWithoutAnyPunctuationOrSpacesThatWillForceTheSystemToUseCharacterBasedSplittingBecauseNoNaturalBreakPointsExist";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // Should have multiple chunks due to forced splitting
        assert!(chunks.len() > 1);

        // Should have forced split chunks
        let forced_chunks = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::ForcedSplit)
            .count();
        assert!(forced_chunks > 0);

        // Verify no chunk exceeds token limit
        for chunk in &chunks {
            assert!(chunk.token_count() <= 8);
        }

        // Verify statistics
        let stats = chunker.statistics();
        assert_eq!(stats.detected_paragraph_count, 1);
        assert_eq!(stats.complete_paragraph_chunks, 0);
        assert!(stats.forced_split_chunks > 0);
    }

    #[test]
    fn test_mixed_chunking_strategies() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 12,
            min_chunk_tokens: 4,
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            max_char_length_fallback: Some(40),
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Complex text that will trigger multiple chunking strategies
        let text = "Short para.\n\n\
                   Another short paragraph here.\n\n\
                   This is a much longer paragraph that contains multiple sentences and will need to be split at the sentence level because it exceeds our token limits.\n\n\
                   VeryLongWordWithoutAnyBreaksAtAllThatWillRequireForcedSplittingBecauseItCannotBeBrokenNaturally\n\n\
                   Final short para.";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // Should have mixed chunk types
        let complete_paragraphs = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::CompleteParagraph)
            .count();
        let merged_paragraphs = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::MergedParagraphs)
            .count();
        let sentence_chunks = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::SentenceBasedSplit)
            .count();
        let forced_chunks = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::ForcedSplit)
            .count();

        // Should have a mix of different chunk types
        let total_strategy_types = [
            complete_paragraphs,
            merged_paragraphs,
            sentence_chunks,
            forced_chunks,
        ]
        .iter()
        .filter(|&&count| count > 0)
        .count();

        assert!(
            total_strategy_types >= 2,
            "Should use multiple chunking strategies"
        );

        // Verify statistics reflect the mixed approach
        let stats = chunker.statistics();
        assert!(stats.detected_paragraph_count >= 4); // At least 4 paragraphs detected
        assert!(stats.total_chunks_created >= 3); // Multiple chunks created

        // Should have used multiple strategies
        let strategies_used = [
            stats.complete_paragraph_chunks > 0,
            stats.merged_paragraph_chunks > 0,
            stats.sentence_based_chunks > 0,
            stats.forced_split_chunks > 0,
        ]
        .iter()
        .filter(|&&used| used)
        .count();

        assert!(
            strategies_used >= 2,
            "Should have used at least 2 different strategies"
        );
    }

    #[test]
    fn test_paragraph_merging_behavior() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 50,
            min_chunk_tokens: 15, // Higher threshold to trigger merging
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config, token_provider, None).unwrap();

        // Create multiple small paragraphs that should be merged
        let text = "Small para one.\n\n\
                   Small para two.\n\n\
                   Small para three.\n\n\
                   Small para four.";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // Should have fewer chunks than paragraphs due to merging
        assert!(chunks.len() < 4);

        // Should have merged paragraph chunks
        let merged_chunks = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::MergedParagraphs)
            .count();
        assert!(merged_chunks > 0);

        // Verify merged chunks contain multiple paragraph separators
        let has_paragraph_breaks = chunks.iter().any(|c| c.content.contains("\n\n"));
        assert!(has_paragraph_breaks);

        // Verify statistics
        let stats = chunker.statistics();
        assert_eq!(stats.detected_paragraph_count, 4);
        assert!(stats.merged_paragraph_chunks > 0);
    }

    #[test]
    fn test_boundary_preservation_quality() {
        let config = HierarchicalChunkingConfig {
            max_chunk_tokens: 15,
            min_chunk_tokens: 5,
            enable_paragraph_merging: true,
            enable_sentence_splitting: true,
            enable_forced_splitting: true,
            preserve_paragraph_boundaries: true,
            ..Default::default()
        };
        let token_provider = MockTokenProvider;
        let mut chunker = HierarchicalChunker::new(config.clone(), token_provider, None).unwrap();

        let text = "Good paragraph here.\n\n\
                   Another good paragraph.\n\n\
                   This is a longer paragraph that will need sentence splitting but should still preserve semantic boundaries where possible.\n\n\
                   Final paragraph.";

        let chunks = chunker.chunk_efficiently(text).unwrap();

        // 1. Verify chunk count is reasonable
        assert!(
            chunks.len() >= 3,
            "Should have at least 3 chunks for this text"
        );
        assert!(
            chunks.len() <= 10,
            "Should not over-fragment into too many chunks"
        );

        // 2. Verify character positions are within text bounds
        let text_len = text.len();
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.char_start <= text_len,
                "Chunk {} start position {} exceeds text length {}",
                i,
                chunk.char_start,
                text_len
            );
            assert!(
                chunk.char_end <= text_len,
                "Chunk {} end position {} exceeds text length {}",
                i,
                chunk.char_end,
                text_len
            );
            assert!(
                chunk.char_start <= chunk.char_end,
                "Chunk {} start position {} should not exceed end position {}",
                i,
                chunk.char_start,
                chunk.char_end
            );
        }

        // 3. Verify chunks are properly ordered by position
        for i in 0..chunks.len() - 1 {
            assert!(
                chunks[i].char_start <= chunks[i + 1].char_start,
                "Chunks should be ordered: chunk {} start {} should be <= chunk {} start {}",
                i,
                chunks[i].char_start,
                i + 1,
                chunks[i + 1].char_start
            );
        }

        // 4. Extract actual content using char positions and verify it matches chunk content
        let text_chars: Vec<char> = text.chars().collect();

        for (i, chunk) in chunks.iter().enumerate() {
            if chunk.char_start < chunk.char_end && chunk.char_end <= text_chars.len() {
                let expected_content: String = text_chars
                    .get(chunk.char_start..chunk.char_end)
                    .unwrap_or(&[])
                    .iter()
                    .collect();

                // Strict validation: content should match positions exactly
                let actual_trimmed = chunk.content.trim();
                let expected_trimmed = expected_content.trim();

                assert!(
                    actual_trimmed == expected_trimmed ||
                    // Allow for minor whitespace differences in processing
                    actual_trimmed.replace(&[' ', '\t', '\n'][..], "") == expected_trimmed.replace(&[' ', '\t', '\n'][..], ""),
                    "Chunk {} content mismatch:\nActual: {:?}\nExpected from positions: {:?}\nPositions: {}-{}",
                    i, chunk.content, expected_content, chunk.char_start, chunk.char_end
                );
            }
        }

        // 5. Verify no gaps or significant overlaps in coverage
        let mut covered_chars = vec![false; text_len];
        for chunk in &chunks {
            for pos in chunk.char_start..chunk.char_end.min(text_len) {
                covered_chars[pos] = true;
            }
        }

        // Count uncovered positions (excluding whitespace-only areas)
        let uncovered_count = covered_chars
            .iter()
            .enumerate()
            .filter(|(i, &covered)| {
                !covered && *i < text_chars.len() && !text_chars[*i].is_whitespace()
            })
            .count();

        // Allow for reasonable gaps between chunks (paragraph breaks, etc.)
        let max_allowed_uncovered = text_len / 5; // Allow up to 20% uncovered
        assert!(
            uncovered_count <= max_allowed_uncovered,
            "Too many uncovered characters: {} out of {} total (max allowed: {})",
            uncovered_count,
            text_len,
            max_allowed_uncovered
        );

        // 6. Verify chunk indices are sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.chunk_index, i,
                "Chunk {} should have index {}, but has {}",
                i, i, chunk.chunk_index
            );
        }

        // 7. Verify all chunks respect token limits
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.tokens.len() <= config.max_chunk_tokens,
                "Chunk {} has {} tokens but max_chunk_tokens is {}. Chunk type: {:?}, Content: {:?}",
                i, chunk.tokens.len(), config.max_chunk_tokens, chunk.chunk_type, chunk.content
            );
        }
    }
}
