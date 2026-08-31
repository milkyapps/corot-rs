#![feature(prelude_import)]
//! `for` with await on the iterable and await in the body.
//!
//! Run: `cargo run -p corot-rs --example for_await`
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;

use corot_macros::corot;
use std::task::Poll;

#[allow(dead_code)]
enum ForRangeCoroutine {
    NotStarted,
    WaitingIter0 {
        __wait: ::core::option::Option<::std::ops::Range<i32>>,
    },
    WaitingN {
        __iter: ::std::ops::Range<i32>,
        i: i32,
        __wait: ::core::option::Option<
            // await in the `in` expression, then await again inside the body
            i32,
        >,
    },
    LoopHead0 {
        __iter: ::std::ops::Range<i32>,
    },
    AfterLoop0 {},
    Finished,
}
enum ForRangeCoroutineRehydration {
    Ok,
}
impl ForRangeCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingIter0 { __wait, .. } => {
                let value =
                    value.downcast_ref::<::std::ops::Range<i32>>().unwrap_or_else(||


                            // 1) wait for the iterable

                            // 2) three body iterations

                            // LoopHead sees end of range → AfterLoop → Finished
                            {
                                ::core::panicking::panic_fmt(format_args!("settle_wait: expected {0}",
                                        ::core::any::type_name::<::std::ops::Range<i32>>()));
                            });
                *__wait = ::core::option::Option::Some(::core::clone::Clone::clone(value));
            }
            Self::WaitingN { __wait, .. } => {
                let value = value.downcast_ref::<i32>().unwrap_or_else(|| {
                    ::core::panicking::panic_fmt(format_args!(
                        "settle_wait: expected {0}",
                        ::core::any::type_name::<i32>()
                    ));
                });
                *__wait = ::core::option::Option::Some(*value);
            }
            _ => {
                ::core::panicking::panic_fmt(format_args!("settle_wait called when not waiting"));
            }
        }
    }
    pub fn rehydrate(&mut self) -> ForRangeCoroutineRehydration {
        ForRangeCoroutineRehydration::Ok
    }
    pub fn get_i(&self) -> ::core::option::Option<&i32> {
        match self {
            Self::WaitingN { i, .. } => ::core::option::Option::Some(i),
            _ => ::core::option::Option::None,
        }
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, ForRangeCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    {
                        ::std::io::_print(format_args!("start\n"));
                    };
                    let _ = 0..3;
                    *self = Self::WaitingIter0 {
                        __wait: ::core::option::Option::None,
                    };
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                }
                Self::WaitingIter0 { __wait } => {
                    let __iterable = __wait.expect("call settle_wait before step");
                    let mut __iter = ::core::iter::IntoIterator::into_iter(__iterable);
                    *self = Self::LoopHead0 { __iter };
                    continue 'step;
                }
                Self::LoopHead0 { mut __iter } => match ::core::iter::Iterator::next(&mut __iter) {
                    ::core::option::Option::Some(i) => {
                        {
                            ::std::io::_print(format_args!("i={0}\n", i));
                        };
                        let _ = ();
                        *self = Self::WaitingN {
                            __iter,
                            i,
                            __wait: ::core::option::Option::None,
                        };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                    ::core::option::Option::None => {
                        *self = Self::AfterLoop0 {};
                        continue 'step;
                    }
                },
                Self::WaitingN {
                    mut __iter,
                    i,
                    __wait,
                } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    {
                        ::std::io::_print(format_args!("i={0} n={1}\n", i, n));
                    };
                    *self = Self::LoopHead0 { __iter };
                    continue 'step;
                }
                Self::AfterLoop0 {} => {
                    {
                        ::std::io::_print(format_args!("done\n"));
                    };
                    *self = Self::Finished;
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
                }
                Self::Finished => {
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
                }
            }
        }
    }
}
fn for_range() -> ForRangeCoroutine {
    ForRangeCoroutine::NotStarted
}
fn main() {
    let mut c = for_range();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&(0..3));
    for expected_i in 0..3 {
        if !#[allow(non_exhaustive_omitted_patterns)]
        match c.step() {
            Ok(Poll::Pending) => true,
            _ => false,
        } {
            ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
        };
        c.settle_wait(&(expected_i * 10));
    }
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
}
