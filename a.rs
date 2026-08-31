#![feature(prelude_import)]
//! Compose `#[corot]` functions via `call::<Child>(…).await`.
//!
//! The child is the generated coroutine enum (not a Rust `Future`). The parent
//! drives `child.step()` / forwards `settle_wait` until `Poll::Ready`.
//!
//! Run: `cargo run -p corot-rs --example compose_await`
extern crate std;
#[prelude_import]
use std::prelude::rust_2021::*;

use corot_macros::corot;
use std::task::Poll;

#[allow(dead_code)]
enum LeafCoroutine {
    NotStarted,
    WaitingN { __wait: ::core::option::Option<i32> },
    Finished,
}
enum LeafCoroutineRehydration {
    Ok,
}
impl LeafCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingN { __wait, .. } => {
                let value =
                    value.downcast_ref::<i32>().unwrap_or_else(||



                            // Enter mid → leaf → leaf's await
                            // leaf
                            // mid's own await
                            // mid
                            // root's second leaf
                            // leaf again
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
    pub fn rehydrate(&mut self) -> LeafCoroutineRehydration {
        LeafCoroutineRehydration::Ok
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, LeafCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    {
                        ::std::io::_print(format_args!("leaf: before\n"));
                    };
                    let _ = ();
                    *self = Self::WaitingN {
                        __wait: ::core::option::Option::None,
                    };
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                }
                Self::WaitingN { __wait } => {
                    let __await_n = __wait.expect("call settle_wait before step");
                    let n: i32 = __await_n;
                    {
                        ::std::io::_print(format_args!("leaf: n={0}\n", n));
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
fn leaf() -> LeafCoroutine {
    LeafCoroutine::NotStarted
}
#[allow(dead_code)]
enum MidCoroutine {
    NotStarted,
    WaitingUnit0 { __child: LeafCoroutine },
    WaitingM { __wait: ::core::option::Option<i32> },
    Finished,
}
enum MidCoroutineRehydration {
    Ok,
}
impl MidCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingUnit0 { __child, .. } => {
                __child.settle_wait(value);
            }
            Self::WaitingM { __wait, .. } => {
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
    pub fn rehydrate(&mut self) -> MidCoroutineRehydration {
        MidCoroutineRehydration::Ok
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, MidCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    {
                        ::std::io::_print(format_args!("mid: before leaf\n"));
                    };
                    let __child = corot_rs::call::<LeafCoroutine>(leaf());
                    *self = Self::WaitingUnit0 { __child };
                    continue 'step;
                }
                Self::WaitingUnit0 { mut __child } => match __child.step() {
                    ::core::result::Result::Ok(::core::task::Poll::Pending) => {
                        *self = Self::WaitingUnit0 { __child };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                    ::core::result::Result::Ok(::core::task::Poll::Ready(__await__unit0)) => {
                        let _: () = __await__unit0;
                        {
                            ::std::io::_print(format_args!("mid: after leaf\n"));
                        };
                        let _ = ();
                        *self = Self::WaitingM {
                            __wait: ::core::option::Option::None,
                        };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                    ::core::result::Result::Err(_) => {
                        *self = Self::WaitingUnit0 { __child };
                        {
                            ::core::panicking::panic_fmt(format_args!(
                                "nested #[corot] rehydration is not supported yet"
                            ));
                        };
                    }
                },
                Self::WaitingM { __wait } => {
                    let __await_m = __wait.expect("call settle_wait before step");
                    let m: i32 = __await_m;
                    {
                        ::std::io::_print(format_args!("mid: own await m={0}\n", m));
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
fn mid() -> MidCoroutine {
    MidCoroutine::NotStarted
}
#[allow(dead_code)]
enum RootCoroutine {
    NotStarted,
    WaitingUnit0 { __child: MidCoroutine },
    WaitingUnit1 { __child: LeafCoroutine },
    Finished,
}
enum RootCoroutineRehydration {
    Ok,
}
impl RootCoroutine {
    pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
        match self {
            Self::WaitingUnit0 { __child, .. } => {
                __child.settle_wait(value);
            }
            Self::WaitingUnit1 { __child, .. } => {
                __child.settle_wait(value);
            }
            _ => {
                ::core::panicking::panic_fmt(format_args!("settle_wait called when not waiting"));
            }
        }
    }
    pub fn rehydrate(&mut self) -> RootCoroutineRehydration {
        RootCoroutineRehydration::Ok
    }
    #[allow(unused_variables)]
    pub fn step(
        &mut self,
    ) -> ::core::result::Result<::core::task::Poll<()>, RootCoroutineRehydration> {
        'step: loop {
            match ::core::mem::replace(self, Self::Finished) {
                Self::NotStarted => {
                    {
                        ::std::io::_print(format_args!("root: start\n"));
                    };
                    let __child = corot_rs::call::<MidCoroutine>(mid());
                    *self = Self::WaitingUnit0 { __child };
                    continue 'step;
                }
                Self::WaitingUnit0 { mut __child } => match __child.step() {
                    ::core::result::Result::Ok(::core::task::Poll::Pending) => {
                        *self = Self::WaitingUnit0 { __child };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                    ::core::result::Result::Ok(::core::task::Poll::Ready(__await__unit0)) => {
                        let _: () = __await__unit0;
                        {
                            ::std::io::_print(format_args!("root: after mid\n"));
                        };
                        let __child = corot_rs::call::<LeafCoroutine>(leaf());
                        *self = Self::WaitingUnit1 { __child };
                        continue 'step;
                    }
                    ::core::result::Result::Err(_) => {
                        *self = Self::WaitingUnit0 { __child };
                        {
                            ::core::panicking::panic_fmt(format_args!(
                                "nested #[corot] rehydration is not supported yet"
                            ));
                        };
                    }
                },
                Self::WaitingUnit1 { mut __child } => match __child.step() {
                    ::core::result::Result::Ok(::core::task::Poll::Pending) => {
                        *self = Self::WaitingUnit1 { __child };
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                    }
                    ::core::result::Result::Ok(::core::task::Poll::Ready(__await__unit1)) => {
                        let _: () = __await__unit1;
                        {
                            ::std::io::_print(format_args!("root: done\n"));
                        };
                        *self = Self::Finished;
                        break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
                    }
                    ::core::result::Result::Err(_) => {
                        *self = Self::WaitingUnit1 { __child };
                        {
                            ::core::panicking::panic_fmt(format_args!(
                                "nested #[corot] rehydration is not supported yet"
                            ));
                        };
                    }
                },
                Self::Finished => {
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
                }
            }
        }
    }
}
fn root() -> RootCoroutine {
    RootCoroutine::NotStarted
}
fn main() {
    {
        ::std::io::_print(format_args!("=== compose root → mid → leaf ===\n"));
    };
    let mut c = root();
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&10i32);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&20i32);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Pending) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Pending))")
    };
    c.settle_wait(&30i32);
    if !#[allow(non_exhaustive_omitted_patterns)]
    match c.step() {
        Ok(Poll::Ready(())) => true,
        _ => false,
    } {
        ::core::panicking::panic("assertion failed: matches!(c.step(), Ok(Poll::Ready(())))")
    };
}
