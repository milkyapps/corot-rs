//! Non-unit return types: `T` and `Result<T, E>`.

use corot_rs::corot;
use std::task::Poll;

#[corot]
async fn add_after_await(base: i32) -> i32 {
    println!("add: before base={base}");
    let n: i32 = ().await;
    println!("add: n={n}");
    base + n
}

#[corot]
async fn early_return_value(flag: bool) -> i32 {
    println!("early: flag={flag}");
    if flag {
        return 7;
    }
    let n: i32 = ().await;
    n + 1
}

#[corot]
async fn result_ok_value() -> Result<i32, &'static str> {
    println!("rok: before");
    let n: i32 = ().await?;
    println!("rok: n={n}");
    Ok(n * 2)
}

#[corot]
async fn result_err_value() -> Result<i32, &'static str> {
    println!("rerr: before");
    let n: i32 = ().await?;
    if n < 0 {
        return Err("negative");
    }
    Ok(n)
}

#[corot]
async fn result_await_err() -> Result<i32, &'static str> {
    println!("raerr: before");
    let n: i32 = ().await?;
    Ok(n)
}

#[test]
fn test_return_value() {
    println!("=== -> i32 trailing ===");
    let mut c = add_after_await(10);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(15))));
    println!();

    println!("=== -> i32 early return ===");
    let mut c = early_return_value(true);
    assert!(matches!(c.step(), Ok(Poll::Ready(7))));

    let mut c = early_return_value(false);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(4))));
    println!();

    println!("=== Result<i32, E> Ok ===");
    let mut c = result_ok_value();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(9));
    assert!(matches!(c.step(), Ok(Poll::Ready(Ok(18)))));
    println!();

    println!("=== Result<i32, E> Err return ===");
    let mut c = result_err_value();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Ok::<i32, &str>(-1));
    assert!(matches!(c.step(), Ok(Poll::Ready(Err("negative")))));
    println!();

    println!("=== Result<i32, E> await? Err ===");
    let mut c = result_await_err();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Err::<i32, &str>("suspended"));
    assert!(matches!(c.step(), Ok(Poll::Ready(Err("suspended")))));
}
