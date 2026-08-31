//! `return` / `return <expr>` before or after an await.
//!
//! Rewritten to finish the coroutine with `Poll::Ready(...)` instead of
//! returning from `step()`.

use corot_rs::corot;
use std::task::Poll;

#[corot]
async fn return_after_await() {
    println!("after: before");
    let a: i32 = ().await;
    println!("after: a={a}");
    if a > 0 {
        println!("after: early return");
        return;
    }
    println!("after: unreachable");
}

#[corot]
async fn return_before_await() {
    println!("before: start");
    if true {
        println!("before: early return");
        return;
    }
    let a: i32 = ().await;
    println!("before: unreachable a={a}");
}

#[corot]
async fn return_fallthrough() {
    println!("fall: before");
    let a: i32 = ().await;
    if a < 0 {
        return;
    }
    println!("fall: continued a={a}");
}

#[corot]
async fn return_err_after() -> Result<(), &'static str> {
    println!("err: before");
    let a: i32 = ().await?;
    println!("err: a={a}");
    if a == 0 {
        return Err("zero");
    }
    Ok(())
}

#[corot]
async fn return_ok_early() -> Result<(), &'static str> {
    println!("ok: before");
    let a: i32 = ().await?;
    if a > 0 {
        println!("ok: early Ok");
        return Ok(());
    }
    Err("non-positive")
}

#[test]
fn test_return_await() {
    println!("=== return after await ===");
    let mut c = return_after_await();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== return before await ===");
    let mut c = return_before_await();
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== fallthrough (no early return) ===");
    let mut c = return_fallthrough();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== return Err after await ===");
    let mut c = return_err_after();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(0));
    assert!(matches!(c.step(), Ok(Poll::Ready(Err("zero")))));
    println!();

    println!("=== return Ok early ===");
    let mut c = return_ok_early();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(9));
    assert!(matches!(c.step(), Ok(Poll::Ready(Ok(())))));
}
