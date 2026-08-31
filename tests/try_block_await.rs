//! General `expr?` and `try { … }` blocks (including with await / `await?`).
//!
//! - In a `Result<(), E>` fn, `expr?` finishes with `corot_rs::Step::Ready(Err(…))`.
//! - `let name: Result<T, E> = try { … }` desugars so `?` targets the block.
//! - `await?` inside `try` settles `Result` and joins the try block on `Err`.
//!
//! Note: `try { }` is still unstable in rustc (soft warning); the macro desugars
//! it away so the generated code is stable.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;


fn ok_i(n: i32) -> Result<i32, &'static str> {
    Ok(n)
}

fn err_i(msg: &'static str) -> Result<i32, &'static str> {
    Err(msg)
}

#[corot]
async fn general_question_ok() -> Result<(), &'static str> {
    println!("qok: start");
    let a: i32 = ok_i(3)?;
    println!("qok: a={a}");
    let b: i32 = ().await;
    let c: i32 = ok_i(b + a)?;
    println!("qok: c={c}");
    Ok(())
}

#[corot]
async fn general_question_err() -> Result<(), &'static str> {
    println!("qerr: start");
    let a: i32 = ().await;
    println!("qerr: a={a}");
    let _b: i32 = err_i("nope")?;
    println!("qerr: unreachable");
    Ok(())
}

#[corot]
async fn sync_try_ok() -> Result<(), &'static str> {
    println!("stry: start");
    let x: Result<i32, &'static str> = try {
        let a: i32 = ok_i(2)?;
        a + 1
    };
    println!("stry: x={x:?}");
    let v: i32 = x?;
    println!("stry: v={v}");
    Ok(())
}

#[corot]
async fn sync_try_err() -> Result<(), &'static str> {
    println!("sterr: start");
    let x: Result<i32, &'static str> = try {
        let _a: i32 = err_i("block")?;
        1
    };
    println!("sterr: x={x:?}");
    Ok(())
}

#[corot]
async fn try_await_ok() -> Result<(), &'static str> {
    println!("taok: start");
    let x: Result<i32, &'static str> = try {
        let n: i32 = ().await?;
        println!("taok: n={n}");
        n + 1
    };
    println!("taok: x={x:?}");
    Ok(())
}

#[corot]
async fn try_await_err() -> Result<(), &'static str> {
    println!("taerr: start");
    let x: Result<i32, &'static str> = try {
        let n: i32 = ().await?;
        println!("taerr: unreachable n={n}");
        n
    };
    println!("taerr: x={x:?}");
    Ok(())
}

#[corot]
async fn try_await_then_question() -> Result<(), &'static str> {
    println!("mix: start");
    let x: Result<i32, &'static str> = try {
        let n: i32 = ().await?;
        ok_i(n * 2)?
    };
    let v: i32 = x?;
    println!("mix: v={v}");
    Ok(())
}

#[test]
fn test_try_block_await() {
    println!("=== general ? ok ===");
    let mut c = general_question_ok();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
    println!();

    println!("=== general ? err after await ===");
    let mut c = general_question_err();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Err("nope")))));
    println!();

    println!("=== sync try ok ===");
    let mut c = sync_try_ok();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
    println!();

    println!("=== sync try err ===");
    let mut c = sync_try_err();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
    println!();

    println!("=== try await ok ===");
    let mut c = try_await_ok();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Ok::<i32, &str>(9));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
    println!();

    println!("=== try await err ===");
    let mut c = try_await_err();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Err::<i32, &str>("suspended"));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
    println!();

    println!("=== try await then ? ===");
    let mut c = try_await_then_question();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Ok::<i32, &str>(5));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(Ok(())))));
}