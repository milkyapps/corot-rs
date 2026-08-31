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

/// Result of `step` on a `#[corot]` coroutine.
///
/// - [`Ready`](Step::Ready): coroutine finished with this value
/// - [`Pending`](Step::Pending): suspended on a typed settle await — call `settle_wait`
/// - [`Effect`](Step::Effect): suspended on an external call (e.g. `send_message(1).await`);
///   the host should perform that call, then `settle_wait` with its return value
#[derive(Debug)]
pub enum Step<T, E = core::convert::Infallible> {
    Ready(T),
    Pending,
    Effect(E),
}

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

/// Type ascription helper for `if let` / `let…else` scrutinees.
///
/// ```ignore
/// if let Some(x) = corot_rs::val::<Option<i32>>(fetch().await) { … }
/// let Some(x) = corot_rs::val::<Option<i32>>(fetch().await) else { return; };
/// ```
#[inline(always)]
pub fn val<T>(value: T) -> T {
    value
}

/// Like [`val`], plus a tuple of **pattern-bound field types** (in pattern order).
///
/// Needed when a suspending `match` arm uses a struct or custom tuple-struct
/// pattern — the macro cannot see field types of `Point` / `Pair` otherwise:
///
/// ```ignore
/// match corot_rs::val_fields::<Point, (i32, i32)>(p) {
///     Point { x, y } => { let n: i32 = ().await; … }
/// }
/// match corot_rs::val_fields::<Pair, (i32, bool)>(pair) {
///     Pair(a, b) => { … await … }
/// }
/// ```
///
/// Slice / array patterns (`[a, b]` on `[T; N]` / `&[T]`) do not need this.
/// Field types can also come from `@` literal hints (`x @ 0`).
#[inline(always)]
pub fn val_fields<T, Fields>(value: T) -> T {
    let _ = core::marker::PhantomData::<Fields>;
    value
}

/// Like [`iter`], plus field types for struct / tuple-struct `for` patterns.
///
/// ```ignore
/// for Point { x, y } in corot_rs::iter_fields::<Vec<Point>, (i32, i32)>(pts) { … }
/// for Pair(a, b) in corot_rs::iter_fields::<Vec<Pair>, (i32, i32)>(pairs) { … }
/// ```
#[inline(always)]
pub fn iter_fields<I: IntoIterator, Fields>(iterable: I) -> I {
    let _ = core::marker::PhantomData::<Fields>;
    iterable
}

/// Type ascription helper for awaiting another `#[corot]` coroutine.
///
/// Proc macros cannot prove a value is a generated coroutine enum, so write:
///
/// ```ignore
/// let _: () = corot_rs::call::<ChildCoroutine>(child()).await;
/// ```
///
/// `C` is the child's coroutine enum type (the return type of the `#[corot]` fn).
/// The parent drives `C::step` / `settle_wait` until `Step::Ready`, then resumes.
/// Child `Step::Effect` values bubble as `ParentEffect::NestedChildCoroutine(…)`.
///
/// This function is the identity: it exists only so the macro can read `C`.
#[inline(always)]
pub fn call<C>(child: C) -> C {
    child
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
