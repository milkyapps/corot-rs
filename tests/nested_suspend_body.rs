//! Nested suspending `if` / `if let` inside `loop` / `while` / `for` / `if` bodies.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;

#[corot]
async fn loop_if_then_and_plain() {
    let flag: bool = true;
    let mut n: i32 = 0;
    loop {
        if flag {
            let x: i32 = ().await;
            println!("then x={x}");
            n += x;
        }
        let y: i32 = ().await;
        println!("after if y={y}");
        n += y;
        if n >= 10 {
            break;
        }
    }
    println!("done n={n}");
}

#[corot]
async fn loop_if_false_skips() {
    let flag: bool = false;
    loop {
        if flag {
            let x: i32 = ().await;
            println!("unreachable x={x}");
        }
        let y: i32 = ().await;
        println!("skipped to y={y}");
        break;
    }
}

#[corot]
async fn loop_if_only() {
    let flag: bool = true;
    loop {
        if flag {
            let x: i32 = ().await;
            println!("only x={x}");
        }
        break;
    }
}

#[corot]
async fn while_nested_if() {
    let mut i: i32 = 0;
    while i < 2 {
        if i == 0 {
            let a: i32 = ().await;
            println!("while then a={a}");
        }
        let b: i32 = ().await;
        println!("while b={b}");
        i += 1;
    }
}

#[corot]
async fn for_nested_if() {
    for i in 0..2 {
        if i == 0 {
            let a: i32 = ().await;
            println!("for then a={a}");
        }
        let b: i32 = ().await;
        println!("for i={i} b={b}");
    }
}

#[corot]
async fn if_nested_if() {
    let outer: bool = true;
    if outer {
        if true {
            let a: i32 = ().await;
            println!("inner a={a}");
        }
        let b: i32 = ().await;
        println!("outer b={b}");
    }
}

#[test]
fn test_nested_suspend_body() {
    println!("=== loop if then + plain ===");
    let mut c = loop_if_then_and_plain();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // x
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // y
    c.settle_wait(&4i32); // n=7
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // x again
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // y
    c.settle_wait(&2i32); // n=11 → break
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== loop if false skips ===");
    let mut c = loop_if_false_skips();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // y only
    c.settle_wait(&9i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== loop if only ===");
    let mut c = loop_if_only();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== while nested if ===");
    let mut c = while_nested_if();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // a
    c.settle_wait(&10i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // b
    c.settle_wait(&11i32);
    // i==1: skip then, only b
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&12i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== for nested if ===");
    let mut c = for_nested_if();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // a
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // b
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // b only (i==1)
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== if nested if ===");
    let mut c = if_nested_if();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // a
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending))); // b
    c.settle_wait(&6i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
