#![feature(prelude_import)]
//! Await inside `if`: condition, then-branch, and else-branch.
//!
//! Run: `cargo run --example if_await`
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;

use corot_macros::corot;
use std::task::Poll;

// --- await in the condition (bare `expr.await` → bool) ---

#[allow(dead_code)]
enum AwaitInCondCoroutine {
    NotStarted,
    WaitingCond0 {
        __wait: ::core::option::Option<bool>,
    },
    Finished,
}
enum AwaitInCondCoroutineRehydration {
    Ok,
}
impl AwaitInCondCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingCond0 { __wait, .. } => {
                let value =
                    value.downcast_ref::<bool>().unwrap_or_else(||
                            // --- await inside then ---


                            // --- await inside else ---


                            // prints before, eval ready()
                            // then + after

                            // else + after

                            // enters then, suspends


                            // else path finishes in one step

                            // enters else, suspends

                            {
                                ::core::panicking::panic_fmt(format_args!("settle_wait: expected {0}",
                                        ::core::any::type_name::<bool>()));
                            });
                *__wait = ::core::option::Option::Some(*value);
            }
            _ => {
                ::core::panicking::panic_fmt(format_args!("settle_wait called when not waiting"));
            }
        }
    }
    pub fn rehydrate(&mut self) -> AwaitInCondCoroutineRehydration {
        AwaitInCondCoroutineRehydration::Ok
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, AwaitInCondCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    {
                        ::std::io::_print(format_args!("cond: before\n"));
                    };
                    let _ = ();
                    *self = Self::WaitingCond0 {
                        __wait: ::core::option::Option::None,
                    };
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                }
                Self::WaitingCond0 { __wait } => {
                    let __await_cond0 = __wait.expect("call settle_wait before step");
                    if __await_cond0 {
                        {
                            ::std::io::_print(format_args!("cond: then\n"));
                        };
                    } else {
                        {
                            ::std::io::_print(format_args!("cond: else\n"));
                        };
                    };
                    {
                        ::std::io::_print(format_args!("cond: after\n"));
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
fn await_in_cond() -> AwaitInCondCoroutine {
    AwaitInCondCoroutine::NotStarted
}
#[allow(dead_code)]
enum AwaitInThenCoroutine {
    NotStarted,
    WaitingN {
        flag: bool,
        __wait: ::core::option::Option<i32>,
    },
    AfterIf0 {
        flag: bool,
    },
    Finished,
}
enum AwaitInThenCoroutineRehydration {
    Ok,
}
impl AwaitInThenCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
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
    pub fn rehydrate(&mut self) -> AwaitInThenCoroutineRehydration {
        AwaitInThenCoroutineRehydration::Ok
    }
    pub fn get_flag(&self) -> ::core::option::Option<&bool> {
        match self {
            Self::WaitingN { flag, .. } => ::core::option::Option::Some(flag),
            Self::AfterIf0 { flag, .. } => ::core::option::Option::Some(flag),
            _ => ::core::option::Option::None,
        }
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, AwaitInThenCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    let flag: bool = true;
                    {
                        ::std::io::_print(format_args!("then: before if\n"));
                    };
                    if flag {
                        {
                            ::std::io::_print(format_args!("then: inside then, before await\n"));
                        };
                        let _ = ();
                        *self = Self::WaitingN {
                            flag,
                            __wait: ::core::option::Option::None,
                        };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    } else {
                        {
                            {
                                ::std::io::_print(format_args!("then: else (no await)\n"));
                            };
                        }
                        *self = Self::AfterIf0 { flag };
                        continue 'step;
                    }
                }
                Self::WaitingN { flag, __wait } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    {
                        ::std::io::_print(format_args!(
                            "then: inside then, after await n={0}\n",
                            n
                        ));
                    };
                    *self = Self::AfterIf0 { flag };
                    continue 'step;
                }
                Self::AfterIf0 { flag } => {
                    {
                        ::std::io::_print(format_args!("then: after if\n"));
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
fn await_in_then() -> AwaitInThenCoroutine {
    AwaitInThenCoroutine::NotStarted
}
#[allow(dead_code)]
enum AwaitInElseCoroutine {
    NotStarted,
    WaitingN {
        flag: bool,
        __wait: ::core::option::Option<i32>,
    },
    AfterIf0 {
        flag: bool,
    },
    Finished,
}
enum AwaitInElseCoroutineRehydration {
    Ok,
}
impl AwaitInElseCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
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
    pub fn rehydrate(&mut self) -> AwaitInElseCoroutineRehydration {
        AwaitInElseCoroutineRehydration::Ok
    }
    pub fn get_flag(&self) -> ::core::option::Option<&bool> {
        match self {
            Self::WaitingN { flag, .. } => ::core::option::Option::Some(flag),
            Self::AfterIf0 { flag, .. } => ::core::option::Option::Some(flag),
            _ => ::core::option::Option::None,
        }
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, AwaitInElseCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    let flag: bool = false;
                    {
                        ::std::io::_print(format_args!("else: before if\n"));
                    };
                    if flag {
                        {
                            ::std::io::_print(format_args!("else: then (no await)\n"));
                        };
                        *self = Self::AfterIf0 { flag };
                        continue 'step;
                    } else {
                        {
                            ::std::io::_print(format_args!("else: inside else, before await\n"));
                        };
                        let _ = ();
                        *self = Self::WaitingN {
                            flag,
                            __wait: ::core::option::Option::None,
                        };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                }
                Self::WaitingN { flag, __wait } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    {
                        ::std::io::_print(format_args!(
                            "else: inside else, after await n={0}\n",
                            n
                        ));
                    };
                    *self = Self::AfterIf0 { flag };
                    continue 'step;
                }
                Self::AfterIf0 { flag } => {
                    {
                        ::std::io::_print(format_args!("else: after if\n"));
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
fn await_in_else() -> AwaitInElseCoroutine {
    AwaitInElseCoroutine::NotStarted
}
fn run_cond() {
    {
        ::std::io::_print(format_args!("=== await in condition ===\n"));
    };
    let mut c = await_in_cond();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&true);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
    {
        ::std::io::_print(format_args!("\n"));
    };
    let mut c = await_in_cond();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&false);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
    {
        ::std::io::_print(format_args!("\n"));
    };
}
fn run_then() {
    {
        ::std::io::_print(format_args!("=== await in then ===\n"));
    };
    let mut c = await_in_then();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&7);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
    {
        ::std::io::_print(format_args!("\n"));
    };
}
#[allow(dead_code)]
enum AwaitInThenSkippedCoroutine {
    NotStarted,
    WaitingN {
        flag: bool,
        __wait: ::core::option::Option<i32>,
    },
    AfterIf0 {
        flag: bool,
    },
    Finished,
}
enum AwaitInThenSkippedCoroutineRehydration {
    Ok,
}
impl AwaitInThenSkippedCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
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
    pub fn rehydrate(&mut self) -> AwaitInThenSkippedCoroutineRehydration {
        AwaitInThenSkippedCoroutineRehydration::Ok
    }
    pub fn get_flag(&self) -> ::core::option::Option<&bool> {
        match self {
            Self::WaitingN { flag, .. } => ::core::option::Option::Some(flag),
            Self::AfterIf0 { flag, .. } => ::core::option::Option::Some(flag),
            _ => ::core::option::Option::None,
        }
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, AwaitInThenSkippedCoroutineRehydration>
    {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    let flag: bool = false;
                    if flag {
                        let _ = ();
                        *self = Self::WaitingN {
                            flag,
                            __wait: ::core::option::Option::None,
                        };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    } else {
                        {
                            {
                                ::std::io::_print(format_args!(
                                    "then-skipped: took else, no await\n"
                                ));
                            };
                        }
                        *self = Self::AfterIf0 { flag };
                        continue 'step;
                    }
                }
                Self::WaitingN { flag, __wait } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    {
                        ::std::io::_print(format_args!("unreachable n={0}\n", n));
                    };
                    *self = Self::AfterIf0 { flag };
                    continue 'step;
                }
                Self::AfterIf0 { flag } => {
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
fn await_in_then_skipped() -> AwaitInThenSkippedCoroutine {
    AwaitInThenSkippedCoroutine::NotStarted
}
fn run_then_skipped() {
    {
        ::std::io::_print(format_args!(
            "=== await in then (else path, no suspend) ===\n"
        ));
    };
    let mut c = await_in_then_skipped();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
    {
        ::std::io::_print(format_args!("\n"));
    };
}
fn run_else() {
    {
        ::std::io::_print(format_args!("=== await in else ===\n"));
    };
    let mut c = await_in_else();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&9);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
    {
        ::std::io::_print(format_args!("\n"));
    };
}
fn main() {
    run_cond();
    run_then();
    run_then_skipped();
    run_else();
}
