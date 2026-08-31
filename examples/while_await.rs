//! `while` / `while let` with await in the condition or the body.
//!
//! Run: `cargo run -p corot-rs --example while_await`

#![allow(unused_mut)]

use corot_macros::corot;
use std::task::Poll;

#[corot]
async fn while_body_await() {
    println!("body: start");
    let mut n: i32 = 0;
    while n < 3 {
        let x: i32 = ().await;
        n += 1;
        println!("body: n={n} x={x}");
    }
    println!("body: done n={n}");
}

#[corot]
async fn while_let_body_await() {
    println!("letbody: start");
    let mut opt: Option<i32> = Some(0);
    while let Some(v) = corot_rs::val::<Option<i32>>(opt) {
        let x: i32 = ().await;
        println!("letbody: v={v} x={x}");
        opt = if v >= 1 { None } else { Some(v + 1) };
    }
    println!("letbody: done");
}

#[corot]
async fn while_cond_await() {
    println!("cond: start");
    let mut left: i32 = 2;
    while ().await {
        left -= 1;
        println!("cond: left={left}");
        if left == 0 {
            break;
        }
    }
    println!("cond: done");
}

#[corot]
async fn while_let_scrut_await() {
    println!("scrut: start");
    let mut step: i32 = 0;
    while let Some(v) = corot_rs::val::<Option<i32>>(().await) {
        step += 1;
        println!("scrut: v={v} step={step}");
        if step >= 2 {
            break;
        }
    }
    println!("scrut: done");
}

#[corot]
async fn while_continue() {
    println!("cont: start");
    let mut i: i32 = 0;
    let mut seen: i32 = 0;
    'w: while i < 5 {
        i += 1;
        let x: i32 = ().await;
        if x < 0 {
            continue 'w;
        }
        seen += 1;
        println!("cont: i={i} x={x} seen={seen}");
    }
    println!("cont: done seen={seen}");
}

fn main() {
    println!("=== while body await ===");
    let mut c = while_body_await();
    for expected in [10, 20, 30] {
        assert!(matches!(c.step(), Ok(Poll::Pending)));
        c.settle_wait(&expected);
    }
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== while let body await ===");
    let mut c = while_let_body_await();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&100i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&200i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== while cond await ===");
    let mut c = while_cond_await();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true); // enters body, left becomes 0, break
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== while let scrutinee await ===");
    let mut c = while_let_scrut_await();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Some(7i32));
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Some(8i32));
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== while continue ===");
    let mut c = while_continue();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&-1i32); // continue
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32); // seen=1
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&-2i32); // continue
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&6i32); // seen=2
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&7i32); // i hits 5 after this path... need to track
    // i starts 0; each LoopHead does i+=1 then await. After 5 iterations i=5, cond fails.
    // Settles: -1,5,-2,6,7 → 5 body entries; after 5th resume i=5, goto head, while i<5 false.
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
