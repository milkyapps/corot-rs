//! Await inside `if`: condition, then-branch, and else-branch.
//!
//! Run: `cargo run --example if_await`

use corot_macros::corot;
use std::task::Poll;

// --- await in the condition (bare `expr.await` → bool) ---

#[corot]
async fn await_in_cond() {
    println!("cond: before");
    if ().await {
        println!("cond: then");
    } else {
        println!("cond: else");
    }
    println!("cond: after");
}

// --- await inside then ---

#[corot]
async fn await_in_then() {
    let flag: bool = true;
    println!("then: before if");
    if flag {
        println!("then: inside then, before await");
        let n: i32 = ().await;
        println!("then: inside then, after await n={n}");
    } else {
        println!("then: else (no await)");
    }
    println!("then: after if");
}

// --- await inside else ---

#[corot]
async fn await_in_else() {
    let flag: bool = false;
    println!("else: before if");
    if flag {
        println!("else: then (no await)");
    } else {
        println!("else: inside else, before await");
        let n: i32 = ().await;
        println!("else: inside else, after await n={n}");
    }
    println!("else: after if");
}

fn run_cond() {
    println!("=== await in condition ===");
    let mut c = await_in_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending))); // prints before, eval ready()
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(())))); // then + after
    println!();

    let mut c = await_in_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(())))); // else + after
    println!();
}

fn run_then() {
    println!("=== await in then ===");
    let mut c = await_in_then();
    assert!(matches!(c.step(), Ok(Poll::Pending))); // enters then, suspends
    c.settle_wait(&7);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

#[corot]
async fn await_in_then_skipped() {
    let flag: bool = false;
    if flag {
        let n: i32 = ().await;
        println!("unreachable n={n}");
    } else {
        println!("then-skipped: took else, no await");
    }
}

fn run_then_skipped() {
    println!("=== await in then (else path, no suspend) ===");
    let mut c = await_in_then_skipped();
    assert!(matches!(c.step(), Ok(Poll::Ready(())))); // else path finishes in one step
    println!();
}

fn run_else() {
    println!("=== await in else ===");
    let mut c = await_in_else();
    assert!(matches!(c.step(), Ok(Poll::Pending))); // enters else, suspends
    c.settle_wait(&9);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn main() {
    run_cond();
    run_then();
    run_then_skipped();
    run_else();
}
