//! Labeled `loop` / `for` with `break` / `continue` (labeled or not).
//!
//! Unlabeled `break`/`continue` inside nested sync loops stay native.
//!
//! Run: `cargo run -p corot-rs --example labeled_loop_await`

#![allow(unused_mut)]

use corot_macros::corot;
use std::task::Poll;

#[corot]
async fn labeled_break() {
    println!("break: start");
    let mut sum: i32 = 0;
    'outer: loop {
        let n: i32 = ().await;
        sum += n;
        println!("break: sum={sum}");
        if sum >= 10 {
            break 'outer;
        }
    }
    println!("break: done sum={sum}");
}

#[corot]
async fn unlabeled_continue() {
    println!("cont: start");
    let mut seen: i32 = 0;
    loop {
        let n: i32 = ().await;
        println!("cont: n={n}");
        if n < 0 {
            continue;
        }
        seen += 1;
        println!("cont: seen={seen}");
        if seen >= 2 {
            break;
        }
    }
    println!("cont: done seen={seen}");
}

#[corot]
async fn labeled_continue() {
    println!("lcont: start");
    let mut ok: i32 = 0;
    'again: loop {
        let n: i32 = ().await;
        if n == 0 {
            println!("lcont: skip zero");
            continue 'again;
        }
        ok += 1;
        println!("lcont: ok={ok} n={n}");
        if ok >= 2 {
            break 'again;
        }
    }
    println!("lcont: done");
}

#[corot]
async fn nested_sync_break_outer() {
    println!("nest: start");
    'outer: loop {
        let n: i32 = ().await;
        println!("nest: n={n}");
        for i in 0..3 {
            if i == 1 && n > 5 {
                println!("nest: break 'outer from for i={i}");
                break 'outer;
            }
            if i == 0 {
                continue; // native for-continue
            }
            println!("nest: for i={i}");
        }
        if n <= 5 {
            break;
        }
    }
    println!("nest: done");
}

#[corot]
async fn labeled_for_break() {
    println!("for: start");
    'items: for idx in 0..5 {
        let n: i32 = ().await;
        println!("for: idx={idx} n={n}");
        if n > 0 {
            break 'items;
        }
    }
    println!("for: done");
}

fn main() {
    println!("=== labeled break ===");
    let mut c = labeled_break();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== unlabeled continue ===");
    let mut c = unlabeled_continue();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&-1i32); // continue
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32); // seen=1
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&-2i32); // continue
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32); // seen=2 → break
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== labeled continue ===");
    let mut c = labeled_continue();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&0i32); // continue 'again
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&1i32); // ok=1
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2i32); // ok=2 → break
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== nested sync + break 'outer ===");
    let mut c = nested_sync_break_outer();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32); // n<=5 → unlabeled break after for
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    let mut c = nested_sync_break_outer();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&9i32); // break 'outer from for
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== labeled for break ===");
    let mut c = labeled_for_break();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&0i32);
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
