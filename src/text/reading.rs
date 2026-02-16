#[cfg(feature = "sudachi")]
pub mod sudachi;

#[cfg(feature = "vibrato")]
pub mod vibrato;

// Backward compatibility: `use text::reading::*` brings sudachi API into scope
#[cfg(feature = "sudachi")]
pub use sudachi::*;
