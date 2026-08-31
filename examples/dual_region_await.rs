//! Await in two regions of one construct.
//!
//! Run: `cargo run -p corot-rs --example dual_region_await`

#![allow(unused_mut)]

use corot_macros::corot;
use std::task::Poll;

#[corot]
async fn while_cond_and_body() {
    let mut n: i32 = 0;
    println!("wcb: start");
    while ().await {
        let x: i32 = ().await;
        n += x;
        println!("wcb: n={n}");
        if n >= 5 {
            break;
        }
    }
    println!("wcb: done n={n}");
}

#[corot]
async fn for_iter_and_multi_body() {
    println!("fim: start");
    for i in corot_rs::iter::<Vec<i32>>(().await) {
        let a: i32 = ().await;
        let b: i32 = ().await;
        println!("fim: i={i} a={a} b={b}");
    }
    println!("fim: done");
}

#[corot]
async fn if_cond_and_then() {
    println!("ict: start");
    if ().await {
        let n: i32 = ().await;
        println!("ict: then n={n}");
    } else {
        println!("ict: else");
    }
    println!("ict: done");
}

#[corot]
async fn if_let_scrut_and_then() {
    println!("ilst: start");
    if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        let n: i32 = ().await;
        println!("ilst: x={x} n={n}");
    } else {
        println!("ilst: none");
    }
    println!("ilst: done");
}

#[corot]
async fn if_cond_and_else() {
    println!("ice: start");
    if ().await {
        println!("ice: then");
    } else {
        let n: i32 = ().await;
        println!("ice: else n={n}");
    }
    println!("ice: done");
}

#[corot]
async fn match_guard_and_body() {
    let x: i32 = 2;
    println!("mgb: start");
    match x {
        0 => println!("mgb: zero"),
        2 if ().await => {
            let n: i32 = ().await;
            println!("mgb: matched n={n}");
        }
        _ => println!("mgb: fallthrough"),
    }
    println!("mgb: done");
}

#[corot]
async fn else_if_cond_and_then() {
    let a: bool = false;
    println!("eict: start");
    if a {
        println!("eict: first");
    } else if ().await {
        let n: i32 = ().await;
        println!("eict: else-if then n={n}");
    } else {
        println!("eict: final else");
    }
    println!("eict: done");
}

#[corot]
async fn else_if_multi_body() {
    let a: i32 = 2;
    println!("eimb: start");
    if a == 0 {
        println!("eimb: first");
    } else if a == 1 {
        let n: i32 = ().await;
        println!("eimb: second n={n}");
    } else if a == 2 {
        let m: i32 = ().await;
        println!("eimb: third m={m}");
    } else {
        println!("eimb: else");
    }
    println!("eimb: done");
}

fn main() {
    println!("=== while cond + body ===");
    let mut c = while_cond_and_body();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32); // n=3
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32); // n=6 → break → done
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== for iter + multi body ===");
    let mut c = for_iter_and_multi_body();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&vec![10, 20]);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== if cond + then ===");
    let mut c = if_cond_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    let mut c = if_cond_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== if let scrut + then ===");
    let mut c = if_let_scrut_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&Some(5i32));
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&9i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    let mut c = if_let_scrut_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== if cond + else ===");
    let mut c = if_cond_and_else();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&11i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    let mut c = if_cond_and_else();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== match guard + body ===");
    let mut c = match_guard_and_body();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&42i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    let mut c = match_guard_and_body();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== else-if cond + then ===");
    let mut c = else_if_cond_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&true);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    let mut c = else_if_cond_and_then();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&false);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));

    println!("=== else-if multi body (exclusive) ===");
    let mut c = else_if_multi_body();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&99i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
