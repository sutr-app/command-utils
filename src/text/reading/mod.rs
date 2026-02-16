#[cfg(feature = "reading-sudachi")]
pub mod sudachi;

#[cfg(feature = "reading-vibrato")]
pub mod vibrato;

// Backward compatibility: `use text::reading::*` brings sudachi API into scope
#[cfg(feature = "reading-sudachi")]
pub use sudachi::*;
