//! Runtime helpers for `#[corot]`.
//!
//! Proc macros cannot tell whether a type implements `serde::Serialize` /
//! `Deserialize`. Wrap locals that must not (or cannot) be serialized in
//! [`SkipSerde`]; the macro emits `#[serde(skip)]` for those fields when the
//! `serde` feature is enabled.

pub use corot_macros::corot;

/// Marker wrapper: captured across await, but omitted from serde.
///
/// On deserialize the field is filled with [`Default::default`], so `T: Default`
/// is required when using the `serde` feature.
pub struct SkipSerde<T>(pub T);

impl<T> SkipSerde<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Default> Default for SkipSerde<T> {
    fn default() -> Self {
        Self(T::default())
    }
}
