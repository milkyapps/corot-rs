//! Runtime helpers for `#[corot]`.
//!
//! Proc macros cannot tell whether a type implements `serde::Serialize` /
//! `Deserialize`. Wrap locals that must not (or cannot) be serialized in
//! [`SkipSerde`]; the macro emits `#[serde(skip)]` for those fields when the
//! `serde` feature is enabled.
//!
//! After deserialize, a skipped field is [`Default`] (`None` inside) and
//! [`SkipSerde::needs_rehydration`] is true until you [`SkipSerde::set`] a value.

pub use corot_macros::corot;

/// Type ascription helper for `#[corot]` `for` loops over arbitrary iterables.
///
/// Proc macros cannot infer `IntoIterator` types, so write:
///
/// ```ignore
/// for x in corot_rs::iter::<Vec<i32>>(v) { … }
/// for x in corot_rs::iter::<Vec<i32>>(fetch().await) { … }
/// ```
///
/// `I` is the type of the `in` expression (and of `settle_wait` when the
/// iterable is awaited). Range literals (`0..3`) still work without this wrapper.
///
/// This function is the identity: it exists only so the macro can read `I`.
#[inline(always)]
pub fn iter<I: IntoIterator>(iterable: I) -> I {
    iterable
}

/// Marker wrapper: captured across await, omitted from serde.
///
/// - [`SkipSerde::new`] / [`SkipSerde::set`]: hydrated value
/// - [`Default`]: empty (`None`) — used after `#[serde(skip)]` deserialize
pub struct SkipSerde<T> {
    value: Option<T>,
}

impl<T> SkipSerde<T> {
    pub fn new(value: T) -> Self {
        Self { value: Some(value) }
    }

    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }

    pub fn needs_rehydration(&self) -> bool {
        self.value.is_none()
    }

    pub fn set(&mut self, value: T) {
        self.value = Some(value);
    }

    pub fn into_inner(self) -> Option<T> {
        self.value
    }
}

impl<T> Default for SkipSerde<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<T> std::ops::Deref for SkipSerde<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
            .as_ref()
            .expect("SkipSerde value needs rehydration before use")
    }
}

impl<T> std::ops::DerefMut for SkipSerde<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
            .as_mut()
            .expect("SkipSerde value needs rehydration before use")
    }
}
