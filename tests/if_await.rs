//! Await inside `if`: condition, then, else, and else-if chains.

use corot_rs::corot;
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

// --- await inside else-if then ---

#[corot]
async fn await_in_else_if() {
    let a: i32 = 1;
    println!("elseif: before");
    if a == 0 {
        println!("elseif: first");
    } else if a == 1 {
        println!("elseif: second, awaiting");
        let n: i32 = ().await;
        println!("elseif: n={n}");
    } else {
        println!("elseif: final else");
    }
    println!("elseif: after");
}

// --- await in final else of an else-if chain ---

#[corot]
async fn await_in_else_if_final() {
    let a: i32 = 2;
    println!("final: before");
    if a == 0 {
        println!("final: first");
    } else if a == 1 {
        println!("final: second");
    } else {
        println!("final: else, awaiting");
        let n: i32 = ().await;
        println!("final: n={n}");
    }
    println!("final: after");
}

// --- skip middle else-if (no suspend) ---

#[corot]
async fn await_in_else_if_skipped() {
    let a: i32 = 0;
    if a == 0 {
        println!("skip: took first branch");
    } else if a == 1 {
        let n: i32 = ().await;
        println!("unreachable n={n}");
    } else {
        println!("unreachable else");
    }
}

// --- await in else-if condition ---

#[corot]
async fn await_in_else_if_cond() {
    let a: bool = false;
    println!("eicond: before");
    if a {
        println!("eicond: first");
    } else if ().await {
        println!("eicond: else-if then");
    } else {
        println!("eicond: else-if else");
    }
    println!("eicond: after");
}

#[corot]
async fn await_in_else_if_cond_nested_skip() {
    let a: bool = false;
    let b: bool = false;
    println!("eicond-nest: before");
    if a {
        println!("eicond-nest: first");
    } else if b {
        println!("eicond-nest: second");
    } else if ().await {
        println!("eicond-nest: else-if then");
    } else {
        println!("eicond-nest: else-if else");
    }
    println!("eicond-nest: after");
}

fn run_cond() {
    println!("=== await in condition ===");
    let mut c = await_in_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    let mut c = await_in_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_then() {
    println!("=== await in then ===");
    let mut c = await_in_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
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
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_else() {
    println!("=== await in else ===");
    let mut c = await_in_else();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&9);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_else_if() {
    println!("=== await in else-if ===");
    let mut c = await_in_else_if();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&42);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_else_if_final() {
    println!("=== await in final else of else-if chain ===");
    let mut c = await_in_else_if_final();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&99);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_else_if_skipped() {
    println!("=== else-if chain (first branch, no suspend) ===");
    let mut c = await_in_else_if_skipped();
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_else_if_cond() {
    println!("=== await in else-if condition (true) ===");
    let mut c = await_in_else_if_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== await in else-if condition (false) ===");
    let mut c = await_in_else_if_cond();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== await in else-if condition nested skip (true) ===");
    let mut c = await_in_else_if_cond_nested_skip();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

#[test]
fn test_if_await() {
    run_cond();
    run_then();
    run_then_skipped();
    run_else();
    run_else_if();
    run_else_if_final();
    run_else_if_skipped();
    run_else_if_cond();
}
