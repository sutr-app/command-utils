/// Sliding window utilities for text processing
///
/// This module provides generic sliding window algorithms that can be used
/// across different text processing systems without dependency on specific
/// tokenizers or embedding frameworks.
use crate::text::chunking::error::{HierarchicalChunkingError, Result};

/// Core sliding window calculation algorithm
pub struct SlidingWindowCalculator;

impl SlidingWindowCalculator {
    /// Calculate sliding window positions with instruction consideration
    ///
    /// This is the main algorithm that determines how to split text into windows
    ///
    /// # Arguments
    /// * `text_length` - Length of the text to be windowed (in tokens)
    /// * `instruction_length` - Length of instruction prefix (in tokens)
    /// * `max_seq_length` - Maximum sequence length allowed
    /// * `window_stride` - Step size between windows
    /// * `min_window_size` - Minimum window size to include
    ///
    /// # Returns
    /// Vector of (start_pos, end_pos) tuples for each window
    pub fn calculate_sliding_windows(
        text_length: usize,
        instruction_length: usize,
        max_seq_length: usize,
        window_stride: usize,
        min_window_size: usize,
    ) -> Result<Vec<(usize, usize)>> {
        // Validate instruction length
        if instruction_length >= max_seq_length {
            return Err(HierarchicalChunkingError::Configuration(format!(
                "Instruction too long: {instruction_length} tokens > max_seq_length {max_seq_length}"
            )));
        }

        let effective_window_size = max_seq_length - instruction_length;

        // Single window case
        if text_length <= effective_window_size {
            return Ok(vec![(0, text_length)]);
        }

        // Multiple windows case - sliding window algorithm
        let mut positions = Vec::new();
        let mut start_pos = 0;
        let mut window_index = 0;

        while start_pos < text_length {
            let end_pos = std::cmp::min(start_pos + effective_window_size, text_length);

            // Window size validation
            let window_size = end_pos - start_pos;
            if window_size < min_window_size && window_index > 0 {
                break;
            }

            positions.push((start_pos, end_pos));

            // Move to next window
            if end_pos >= text_length {
                break;
            }

            start_pos += window_stride;
            window_index += 1;
        }

        Ok(positions)
    }

    /// Calculate window weights giving more weight to middle windows
    ///
    /// This is useful for weighted averaging of embeddings from multiple windows
    pub fn calculate_window_weights(num_windows: usize) -> Vec<f32> {
        let mut weights = vec![1.0f32; num_windows];

        // Give slightly more weight to middle windows for better representation
        if num_windows > 2 {
            for (i, weight) in weights.iter_mut().enumerate() {
                let position = i as f32 / (num_windows - 1) as f32; // 0.0 to 1.0
                let distance_from_center = (position - 0.5).abs() * 2.0; // 0.0 to 1.0
                *weight = 1.0 + (1.0 - distance_from_center) * 0.2; // 1.0 to 1.2
            }
        }

        weights
    }
}

/// Merge strategies for combining results from multiple windows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeStrategy {
    /// Simple average of all windows
    Average,
    /// Weighted average (more weight to middle windows)
    #[default]
    WeightedAverage,
    /// Use only the first window
    FirstWindow,
    /// Use only the last window
    LastWindow,
}

/// Generic embedding merger for sliding window results
pub struct EmbeddingMerger;

impl EmbeddingMerger {
    /// Merge embeddings from multiple windows
    pub fn merge_embeddings(
        embeddings: &[Vec<f32>],
        merge_strategy: MergeStrategy,
    ) -> Result<Vec<f32>> {
        if embeddings.is_empty() {
            return Err(HierarchicalChunkingError::Validation(
                "No embeddings to merge".to_string(),
            ));
        }

        if embeddings.len() == 1 {
            return Ok(embeddings[0].clone());
        }

        let embedding_dim = embeddings[0].len();

        // Validate all embeddings have same dimension
        for (i, emb) in embeddings.iter().enumerate() {
            if emb.len() != embedding_dim {
                return Err(HierarchicalChunkingError::Validation(format!(
                    "Embedding {} has dimension {} but expected {}",
                    i,
                    emb.len(),
                    embedding_dim
                )));
            }
        }

        match merge_strategy {
            MergeStrategy::Average => Self::merge_by_average(embeddings),
            MergeStrategy::WeightedAverage => Self::merge_by_weighted_average(embeddings),
            MergeStrategy::FirstWindow => Ok(embeddings[0].clone()),
            MergeStrategy::LastWindow => Ok(embeddings[embeddings.len() - 1].clone()),
        }
    }

    /// Merge embeddings by simple averaging
    pub fn merge_by_average(embeddings: &[Vec<f32>]) -> Result<Vec<f32>> {
        let embedding_dim = embeddings[0].len();
        let mut merged = vec![0.0f32; embedding_dim];

        for embedding in embeddings {
            for (i, &value) in embedding.iter().enumerate() {
                merged[i] += value;
            }
        }

        // Average
        let num_embeddings = embeddings.len() as f32;
        for value in merged.iter_mut() {
            *value /= num_embeddings;
        }

        Ok(merged)
    }

    /// Merge embeddings by weighted averaging (giving more weight to middle windows)
    pub fn merge_by_weighted_average(embeddings: &[Vec<f32>]) -> Result<Vec<f32>> {
        let embedding_dim = embeddings[0].len();
        let mut merged = vec![0.0f32; embedding_dim];

        // Simple weight scheme: give more weight to middle windows
        let weights = SlidingWindowCalculator::calculate_window_weights(embeddings.len());
        let total_weight: f32 = weights.iter().sum();

        for (embedding, weight) in embeddings.iter().zip(weights.iter()) {
            for (i, &value) in embedding.iter().enumerate() {
                merged[i] += value * weight;
            }
        }

        // Normalize by total weight
        for value in merged.iter_mut() {
            *value /= total_weight;
        }

        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sliding_window_algorithm() {
        // Test case 1: Single window (text fits within effective window size)
        let result_single = SlidingWindowCalculator::calculate_sliding_windows(
            400, // text_length
            10,  // instruction_length
            512, // max_seq_length
            256, // window_stride
            64,  // min_window_size
        );
        assert!(result_single.is_ok());
        let windows_single = result_single.unwrap();
        assert_eq!(windows_single.len(), 1);
        assert_eq!(windows_single[0], (0, 400));

        // Test case 2: Multiple windows (1000 tokens, instruction 2 tokens, max 512)
        let result_multiple = SlidingWindowCalculator::calculate_sliding_windows(
            1000, // text_length
            2,    // instruction_length
            512,  // max_seq_length
            256,  // window_stride
            64,   // min_window_size
        );
        assert!(result_multiple.is_ok());
        let windows_multiple = result_multiple.unwrap();
        assert_eq!(windows_multiple.len(), 3);
        assert_eq!(windows_multiple[0], (0, 510)); // Window 0: [0..510)
        assert_eq!(windows_multiple[1], (256, 766)); // Window 1: [256..766)
        assert_eq!(windows_multiple[2], (512, 1000)); // Window 2: [512..1000)

        // Test case 3: Instruction too long (should return error)
        let result_error = SlidingWindowCalculator::calculate_sliding_windows(
            100, // text_length
            600, // instruction_length (too long)
            512, // max_seq_length
            256, // window_stride
            64,  // min_window_size
        );
        assert!(result_error.is_err());

        // Test case 4: Small final window skipping
        let result_skip = SlidingWindowCalculator::calculate_sliding_windows(
            800, // text_length
            2,   // instruction_length
            512, // max_seq_length
            256, // window_stride
            100, // min_window_size (larger to trigger skipping)
        );
        assert!(result_skip.is_ok());
        let windows_skip = result_skip.unwrap();
        // Window 2 would be [512..800) = 288 tokens, which is > 100, so it should be included
        assert_eq!(windows_skip.len(), 3);
        assert_eq!(windows_skip[2], (512, 800));
    }

    #[test]
    fn test_merge_embeddings() {
        // Test embedding merge functionality
        let embeddings = vec![
            vec![1.0, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
        ];

        // Test average merge
        let result_avg = EmbeddingMerger::merge_embeddings(&embeddings, MergeStrategy::Average);
        assert!(result_avg.is_ok());
        let merged_avg = result_avg.unwrap();
        assert_eq!(merged_avg, vec![4.0, 5.0, 6.0]); // (1+4+7)/3, (2+5+8)/3, (3+6+9)/3

        // Test weighted average merge
        let result_weighted =
            EmbeddingMerger::merge_embeddings(&embeddings, MergeStrategy::WeightedAverage);
        assert!(result_weighted.is_ok());
        let merged_weighted = result_weighted.unwrap();
        // Should be weighted differently (more weight to middle window)
        assert_eq!(merged_weighted.len(), 3);

        // Test first window strategy
        let result_first =
            EmbeddingMerger::merge_embeddings(&embeddings, MergeStrategy::FirstWindow);
        assert!(result_first.is_ok());
        let merged_first = result_first.unwrap();
        assert_eq!(merged_first, vec![1.0, 2.0, 3.0]);

        // Test last window strategy
        let result_last = EmbeddingMerger::merge_embeddings(&embeddings, MergeStrategy::LastWindow);
        assert!(result_last.is_ok());
        let merged_last = result_last.unwrap();
        assert_eq!(merged_last, vec![7.0, 8.0, 9.0]);

        // Test single embedding
        let single_embedding = vec![vec![1.0, 2.0, 3.0]];
        let result_single =
            EmbeddingMerger::merge_embeddings(&single_embedding, MergeStrategy::Average);
        assert!(result_single.is_ok());
        let merged_single = result_single.unwrap();
        assert_eq!(merged_single, vec![1.0, 2.0, 3.0]);

        // Test empty embeddings (should error)
        let empty_embeddings: Vec<Vec<f32>> = vec![];
        let result_empty =
            EmbeddingMerger::merge_embeddings(&empty_embeddings, MergeStrategy::Average);
        assert!(result_empty.is_err());

        // Test mismatched dimensions (should error)
        let mismatched_embeddings = vec![
            vec![1.0, 2.0, 3.0],
            vec![4.0, 5.0], // Different dimension
        ];
        let result_mismatch =
            EmbeddingMerger::merge_embeddings(&mismatched_embeddings, MergeStrategy::Average);
        assert!(result_mismatch.is_err());
    }

    #[test]
    fn test_window_weights_calculation() {
        // Test window weight calculation

        // Single window
        let weights_1 = SlidingWindowCalculator::calculate_window_weights(1);
        assert_eq!(weights_1, vec![1.0]);

        // Two windows
        let weights_2 = SlidingWindowCalculator::calculate_window_weights(2);
        assert_eq!(weights_2, vec![1.0, 1.0]);

        // Three windows (middle should have higher weight)
        let weights_3 = SlidingWindowCalculator::calculate_window_weights(3);
        assert_eq!(weights_3.len(), 3);
        assert!(weights_3[1] > weights_3[0]); // Middle > first
        assert!(weights_3[1] > weights_3[2]); // Middle > last

        // Five windows (middle should have highest weight)
        let weights_5 = SlidingWindowCalculator::calculate_window_weights(5);
        assert_eq!(weights_5.len(), 5);
        assert!(weights_5[2] > weights_5[0]); // Center > edge
        assert!(weights_5[2] > weights_5[4]); // Center > edge
        assert!(weights_5[1] > weights_5[0]); // Closer to center > edge
        assert!(weights_5[3] > weights_5[4]); // Closer to center > edge
    }
}
