//! Loop-as-expression: `let x: T = loop { …; break value }` with await.
//!
//! Also covers labeled `break 'lab value` and using the break value after
//! a later await.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;


#[corot]
async fn break_value() {
    println!("break: start");
    let x: i32 = loop {
        let n: i32 = ().await;
        println!("break: n={n}");
        if n > 0 {
            break n + 1;
        }
    };
    println!("break: x={x}");
}

#[corot]
async fn labeled_break_value() {
    println!("lab: start");
    let x: i32 = 'out: loop {
        let n: i32 = ().await;
        println!("lab: n={n}");
        if n >= 10 {
            break 'out n;
        }
        if n < 0 {
            continue 'out;
        }
    };
    println!("lab: x={x}");
}

#[corot]
async fn break_before_await() {
    println!("early: start");
    let ready: i32 = 1;
    let x: i32 = loop {
        if ready > 0 {
            break 42;
        }
        let n: i32 = ().await;
        break n;
    };
    println!("early: x={x}");
}

#[corot]
async fn break_value_then_await() {
    println!("then: start");
    let x: i32 = loop {
        let n: i32 = ().await;
        if n > 0 {
            break n;
        }
    };
    println!("then: x={x}");
    let y: i32 = ().await;
    println!("then: x+y={}", x + y);
}

#[corot]
async fn break_from_nested_for() {
    println!("nest: start");
    let x: i32 = 'outer: loop {
        let n: i32 = ().await;
        for i in 0..3 {
            if i == 2 {
                break 'outer n + i;
            }
        }
    };
    println!("nest: x={x}");
}

#[test]
fn test_loop_break_value_await() {
    println!("=== break value ===");
    let mut c = break_value();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&-1i32); // continue looping
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== labeled break value ===");
    let mut c = labeled_break_value();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&-3i32); // continue
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&5i32); // keep going (5 < 10)
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&12i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== break before await ===");
    let mut c = break_before_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== break value then await ===");
    let mut c = break_value_then_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== break from nested for ===");
    let mut c = break_from_nested_for();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&10i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}