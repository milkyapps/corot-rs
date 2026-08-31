#![feature(prelude_import)]
//! Simple `loop` with one await inside (and `break`).
//!
//! Run: `cargo run --example loop_await`
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;

use corot_macros::corot;
use std::task::Poll;

#[allow(dead_code)]
enum CountLoopCoroutine {
    NotStarted,
    WaitingN {
        sum: i32,
        __wait: ::core::option::Option<i32>,
    },
    LoopHead0 {
        sum: i32,
    },
    AfterLoop0 {
        sum: i32,
    },
    Finished,
}
enum CountLoopCoroutineRehydration {
    Ok,
}
impl CountLoopCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingN { __wait, .. } => {
                let value =
                    value.downcast_ref::<i32>().unwrap_or_else(||

                            // iteration 1

                            // iteration 2

                            // iteration 3 → sum becomes 10 → break → done

                            {
                                ::core::panicking::panic_fmt(format_args!("settle_wait: expected {0}",
                                        ::core::any::type_name::<i32>()));
                            });
                *__wait = ::core::option::Option::Some(*value);
            }
            _ => {
                ::core::panicking::panic_fmt(format_args!("settle_wait called when not waiting"));
            }
        }
    }
    pub fn rehydrate(&mut self) -> CountLoopCoroutineRehydration {
        CountLoopCoroutineRehydration::Ok
    }
    pub fn get_sum(&self) -> ::core::option::Option<&i32> {
        match self {
            Self::WaitingN { sum, .. } => ::core::option::Option::Some(sum),
            Self::AfterLoop0 { sum, .. } => ::core::option::Option::Some(sum),
            Self::LoopHead0 { sum, .. } => ::core::option::Option::Some(sum),
            _ => ::core::option::Option::None,
        }
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, CountLoopCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    let mut sum: i32 = 0;
                    {
                        ::std::io::_print(format_args!("start\n"));
                    };
                    *self = Self::LoopHead0 { sum };
                    continue 'step;
                }
                Self::LoopHead0 { mut sum } => {
                    {
                        ::std::io::_print(format_args!("sum={0}\n", sum));
                    };
                    let _ = ();
                    *self = Self::WaitingN {
                        sum,
                        __wait: ::core::option::Option::None,
                    };
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                }
                Self::WaitingN { mut sum, __wait } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    sum += n;
                    {
                        ::std::io::_print(format_args!("added n={0}, sum={1}\n", n, sum));
                    };
                    if sum >= 10 {
                        *self = Self::AfterLoop0 { sum };
                        continue 'step;
                    }
                    *self = Self::LoopHead0 { sum };
                    continue 'step;
                }
                Self::AfterLoop0 { mut sum } => {
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
fn count_loop() -> CountLoopCoroutine {
    CountLoopCoroutine::NotStarted
}
fn main() {
    let mut c = count_loop();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&3);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&4);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&3);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
}
