//! Await in `match`: scrutinee, arm body, and guard.
//!
//! Rust allows `.await` in match guards; corot supports that as a bool settle.

use corot_rs::corot;
use std::task::Poll;

// --- await on the scrutinee ---

#[corot]
async fn await_in_scrutinee() {
    println!("scrut: before");
    match ().await {
        0 => println!("scrut: zero"),
        1 => println!("scrut: one"),
        n => println!("scrut: other {n}"),
    }
    println!("scrut: after");
}

// --- await inside one arm ---

#[corot]
async fn await_in_arm() {
    let x: i32 = 1;
    println!("arm: before");
    match x {
        0 => println!("arm: zero (no await)"),
        1 => {
            println!("arm: one, before await");
            let n: i32 = ().await;
            println!("arm: one, after await n={n}");
        }
        _ => println!("arm: other"),
    }
    println!("arm: after");
}

#[corot]
async fn await_in_arm_skipped() {
    let x: i32 = 0;
    match x {
        0 => println!("arm-skipped: took sync arm"),
        _ => {
            let n: i32 = ().await;
            println!("unreachable n={n}");
        }
    }
}

// --- await in a match guard ---

#[corot]
async fn await_in_guard() {
    let x: i32 = 2;
    println!("guard: before");
    match x {
        0 => println!("guard: zero"),
        // refutable pattern + await guard; false falls through to `_`
        2 if ().await => println!("guard: matched two"),
        _ => println!("guard: fallthrough"),
    }
    println!("guard: after");
}

#[corot]
async fn await_in_guard_binding() {
    let x: i32 = 5;
    match x {
        // literal arm supplies the i32 type hint; binding is re-matched after settle
        n @ 5 if ().await => println!("guard-bind: n={n}"),
        _ => println!("guard-bind: no"),
    }
}

fn run_scrutinee() {
    println!("=== await in scrutinee ===");
    let mut c = await_in_scrutinee();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&1);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    let mut c = await_in_scrutinee();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&7);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_arm() {
    println!("=== await in arm ===");
    let mut c = await_in_arm();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&42);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_arm_skipped() {
    println!("=== await in arm (sync path, no suspend) ===");
    let mut c = await_in_arm_skipped();
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

fn run_guard() {
    println!("=== await in guard (true) ===");
    let mut c = await_in_guard();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== await in guard (false → fallthrough) ===");
    let mut c = await_in_guard();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== await in guard with binding ===");
    let mut c = await_in_guard_binding();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();
}

#[test]
fn test_match_await() {
    run_scrutinee();
    run_arm();
    run_arm_skipped();
    run_guard();
}
