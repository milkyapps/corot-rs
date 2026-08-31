//! `await?` with early `Err` join.
//!
//! The async fn must return `Result<(), E>`. Settling an `Err` finishes the
//! coroutine with `Poll::Ready(Err(...))` without running later statements.

use corot_rs::corot;
use std::task::Poll;

#[corot]
async fn try_ok_path() -> Result<(), &'static str> {
    println!("ok: before");
    let a: i32 = ().await?;
    println!("ok: a={a}");
    let b: i32 = ().await?;
    println!("ok: b={b}");
    println!("ok: done");
    Ok(())
}

#[corot]
async fn try_err_first() -> Result<(), &'static str> {
    println!("err1: before");
    let a: i32 = ().await?;
    println!("err1: unreachable a={a}");
    Ok(())
}

#[corot]
async fn try_err_second() -> Result<(), &'static str> {
    println!("err2: before");
    let a: i32 = ().await?;
    println!("err2: a={a}");
    let b: i32 = ().await?;
    println!("err2: unreachable b={b}");
    Ok(())
}

#[test]
fn test_try_await() {
    println!("=== Ok path ===");
    let mut c = try_ok_path();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(1));
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(2));
    assert!(matches!(c.step(), Ok(Poll::Ready(Ok(())))));
    println!();

    println!("=== Err on first await ===");
    let mut c = try_err_first();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Err::<i32, &str>("boom"));
    assert!(matches!(c.step(), Ok(Poll::Ready(Err("boom")))));
    println!("err1: finished with boom");
    println!();

    println!("=== Err on second await ===");
    let mut c = try_err_second();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(7));
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Err::<i32, &str>("later"));
    assert!(matches!(c.step(), Ok(Poll::Ready(Err("later")))));
    println!("err2: finished with later");
}
