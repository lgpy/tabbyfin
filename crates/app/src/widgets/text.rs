//! Text for cards and pages, and the loading state pages keep. L1 brings `main`'s text helpers
//! from `components.rs` here (card text, episode labels, runtimes, sizes) with their tests.

/// What a page shows while its data loads, once it's there, or after the request failed (with
/// the message for the user).
#[derive(Debug, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Loaded(T),
    Failed(String),
}
