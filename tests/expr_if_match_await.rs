//! Expression-position `if` / `match` with await:
//! `let v: T = if … { … await …; value } else { … }`.

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;

#[corot]
async fn if_then_expr() {
    let c: bool = true;
    let v: i32 = if c {
        let x: i32 = ().await;
        x + 1
    } else {
        0
    };
    println!("if_then_expr v={v}");
}

#[corot]
async fn if_else_expr() {
    let c: bool = false;
    let v: i32 = if c {
        let x: i32 = ().await;
        x + 1
    } else {
        0
    };
    println!("if_else_expr v={v}");
}

#[corot]
async fn if_expr_then_await() {
    let c: bool = true;
    let v: i32 = if c {
        let x: i32 = ().await;
        x + 1
    } else {
        0
    };
    let y: i32 = ().await;
    println!("if_expr_then_await v+y={}", v + y);
}

#[corot]
async fn if_else_await_expr() {
    let c: bool = false;
    let v: i32 = if c {
        1
    } else {
        let x: i32 = ().await;
        x + 2
    };
    println!("if_else_await_expr v={v}");
}

#[corot]
async fn match_arm_expr() {
    let k: i32 = 1;
    let v: i32 = match k {
        0 => 10,
        1 => {
            let x: i32 = ().await;
            x + 20
        }
        _ => 30,
    };
    println!("match_arm_expr v={v}");
}

#[corot]
async fn match_other_arm_expr() {
    let k: i32 = 0;
    let v: i32 = match k {
        0 => 10,
        1 => {
            let x: i32 = ().await;
            x + 20
        }
        _ => 30,
    };
    println!("match_other_arm_expr v={v}");
}

#[test]
fn test_expr_if_match_await() {
    println!("=== if then expr ===");
    let mut c = if_then_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== if else expr (skip await) ===");
    let mut c = if_else_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== if expr then later await ===");
    let mut c = if_expr_then_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== if else await expr ===");
    let mut c = if_else_await_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match arm expr ===");
    let mut c = match_arm_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match other arm expr ===");
    let mut c = match_other_arm_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
