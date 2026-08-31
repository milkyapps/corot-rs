//! Labeled blocks: `let x: T = 'a: { …; break 'a value }` with await inside.
//!
//! Fallthrough trailing expressions and early `break 'label` (before the await)
//! are supported. `continue` cannot target a block label.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;
use std::task::Poll;

#[corot]
async fn break_with_value() {
    println!("break: start");
    let x: i32 = 'a: {
        let n: i32 = ().await;
        println!("break: n={n}");
        break 'a n + 1;
    };
    println!("break: x={x}");
}

#[corot]
async fn fallthrough_value() {
    println!("fall: start");
    let x: i32 = 'out: {
        let n: i32 = ().await;
        println!("fall: n={n}");
        n * 2
    };
    println!("fall: x={x}");
}

#[corot]
async fn early_break_skips_await() {
    println!("early: start");
    let flag: i32 = 1;
    let x: i32 = 'blk: {
        if flag > 0 {
            break 'blk 7;
        }
        let n: i32 = ().await;
        break 'blk n;
    };
    println!("early: x={x}");
}

#[corot]
async fn stmt_labeled_block() {
    println!("stmt: start");
    'work: {
        let n: i32 = ().await;
        println!("stmt: n={n}");
        if n < 0 {
            break 'work;
        }
        println!("stmt: positive");
    };
    println!("stmt: done");
}

#[corot]
async fn break_from_nested_for() {
    println!("nest: start");
    let x: i32 = 'outer: {
        let n: i32 = ().await;
        println!("nest: n={n}");
        for i in 0..3 {
            if i == 1 {
                break 'outer n + i;
            }
        }
        n
    };
    println!("nest: x={x}");
}

#[test]
fn test_labeled_block_await() {
    println!("=== break with value ===");
    let mut c = break_with_value();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== fallthrough value ===");
    let mut c = fallthrough_value();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== early break skips await ===");
    let mut c = early_break_skips_await();
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== stmt labeled block ===");
    let mut c = stmt_labeled_block();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&-1i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    let mut c = stmt_labeled_block();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    println!();

    println!("=== break from nested for ===");
    let mut c = break_from_nested_for();
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&9i32);
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}