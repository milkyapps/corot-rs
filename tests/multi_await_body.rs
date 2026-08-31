//! Multiple typed-let awaits inside one `loop` / `if` / `for` / `match` body.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;
use std::task::Poll;

#[corot]
async fn multi_loop() {
    println!("loop: start");
    let mut sum: i32 = 0;
    loop {
        let a: i32 = ().await;
        let b: i32 = ().await;
        sum += a + b;
        println!("loop: a={a} b={b} sum={sum}");
        if sum >= 10 {
            break;
        }
    }
    println!("loop: done sum={sum}");
}

#[corot]
async fn multi_if() {
    println!("if: start");
    let flag: bool = true;
    if flag {
        let a: i32 = ().await;
        println!("if: a={a}");
        let b: i32 = ().await;
        println!("if: b={b} sum={}", a + b);
    } else {
        println!("if: else");
    }
    println!("if: done");
}

#[corot]
async fn multi_for() {
    println!("for: start");
    for i in 0..2 {
        let a: i32 = ().await;
        let b: i32 = ().await;
        println!("for: i={i} a={a} b={b}");
    }
    println!("for: done");
}

#[corot]
async fn multi_match() {
    println!("match: start");
    let which: i32 = 1;
    match which {
        1 => {
            let a: i32 = ().await;
            let b: i32 = ().await;
            println!("match: a={a} b={b}");
        }
        _ => println!("match: other"),
    }
    println!("match: done");
}

#[test]
fn test_multi_await_body() {
    println!("=== multi loop ===");
    let mut c = multi_loop();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32); // sum=7
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2i32); // sum=11 → break
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== multi if ===");
    let mut c = multi_if();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== multi for ===");
    let mut c = multi_for();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== multi match ===");
    let mut c = multi_match();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&8i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&9i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}