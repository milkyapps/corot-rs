//! `if let` and `let…else` with await.
//!
//! Scrutinee types use `corot_rs::val::<T>(…)` (or literal patterns like `Some(0)`).
//!
//! Run: `cargo run --example if_let_await`

use corot_macros::corot;
use std::task::Poll;

// --- if let: await on scrutinee ---

#[corot]
async fn if_let_scrutinee() {
    println!("scrut: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        println!("scrut: some {x}");
    } else {
        println!("scrut: none");
    }
    println!("scrut: after");
}

// --- if let: await in then ---

#[corot]
async fn if_let_then() {
    let opt: Option<i32> = Some(3);
    println!("then: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(opt) {
        println!("then: got x={x}, awaiting");
        let n: i32 = ().await;
        println!("then: x={x} n={n}");
    } else {
        println!("then: none");
    }
    println!("then: after");
}

// --- if let: await in else ---

#[corot]
async fn if_let_else() {
    let opt: Option<i32> = None;
    println!("else: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(opt) {
        println!("else: unreachable {x}");
    } else {
        println!("else: none path, awaiting");
        let n: i32 = ().await;
        println!("else: n={n}");
    }
    println!("else: after");
}

// --- let…else: await in initializer ---

#[corot]
async fn let_else_init() {
    println!("init: before");
    let Some(x) = corot_rs::val::<Option<i32>>(().await) else {
        println!("init: else diverged");
        return;
    };
    println!("init: x={x}");
    println!("init: after");
}

// --- let…else: await in else block ---

#[corot]
async fn let_else_diverge() {
    let opt: Option<i32> = None;
    println!("diverge: before");
    let Some(x) = corot_rs::val::<Option<i32>>(opt) else {
        println!("diverge: in else, awaiting");
        let n: i32 = ().await;
        println!("diverge: n={n}, returning");
        return;
    };
    println!("diverge: unreachable x={x}");
}

#[corot]
async fn let_else_success() {
    let opt: Option<i32> = Some(9);
    let Some(x) = corot_rs::val::<Option<i32>>(opt) else {
        let n: i32 = ().await;
        println!("unreachable n={n}");
        return;
    };
    println!("success: x={x}");
}

fn main() {
    println!("=== if let scrutinee ===");
    let mut c = if_let_scrutinee();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Some(7));
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    let mut c = if_let_scrutinee();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== if let then ===");
    let mut c = if_let_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&11);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== if let else ===");
    let mut c = if_let_else();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&22);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== let else init (Some) ===");
    let mut c = let_else_init();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Some(5));
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== let else init (None → return) ===");
    let mut c = let_else_init();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== let else diverge await ===");
    let mut c = let_else_diverge();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&33);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== let else success (no suspend) ===");
    let mut c = let_else_success();
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
