//! Multi-level nested `if`, else awaits, nested match/for/while/loop.

#![allow(unused_mut, unreachable_code, dead_code)]

use corot_rs::corot;

#[corot]
async fn multi_level_if() {
    let a: bool = true;
    let b: bool = true;
    loop {
        if a {
            if b {
                let x: i32 = ().await;
                println!("multi x={x}");
            }
        }
        break;
    }
}

#[corot]
async fn multi_level_if_false_outer() {
    let a: bool = false;
    let b: bool = true;
    loop {
        if a {
            if b {
                let x: i32 = ().await;
                println!("unreachable x={x}");
            }
        }
        let y: i32 = ().await;
        println!("skipped to y={y}");
        break;
    }
}

#[corot]
async fn nested_if_else_await() {
    let c: bool = false;
    loop {
        if c {
            println!("then sync");
        } else {
            let x: i32 = ().await;
            println!("else x={x}");
        }
        break;
    }
}

#[corot]
async fn nested_if_then_and_else() {
    let c: bool = true;
    loop {
        if c {
            let a: i32 = ().await;
            println!("both then a={a}");
        } else {
            let b: i32 = ().await;
            println!("both else b={b}");
        }
        break;
    }
}

#[corot]
async fn nested_if_then_and_else_false() {
    let c: bool = false;
    loop {
        if c {
            let a: i32 = ().await;
            println!("both then a={a}");
        } else {
            let b: i32 = ().await;
            println!("both else b={b}");
        }
        break;
    }
}

#[corot]
async fn nested_else_if_await() {
    let a: i32 = 2;
    loop {
        if a == 0 {
            println!("zero");
        } else if a == 2 {
            let x: i32 = ().await;
            println!("elif x={x}");
        } else {
            println!("other");
        }
        break;
    }
}

#[corot]
async fn nested_match_in_loop() {
    let k: i32 = 1;
    loop {
        match k {
            1 => {
                let a: i32 = ().await;
                println!("arm1 a={a}");
            }
            _ => {
                let b: i32 = ().await;
                println!("arm_other b={b}");
            }
        }
        break;
    }
}

#[corot]
async fn nested_for_in_loop() {
    loop {
        for i in 0..2 {
            let x: i32 = ().await;
            println!("for i={i} x={x}");
        }
        break;
    }
}

#[test]
fn test_nested_suspend_rich() {
    let mut c = multi_level_if();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = multi_level_if_false_outer();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_if_else_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_if_then_and_else();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&3i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_if_then_and_else_false();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_else_if_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_match_in_loop();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    let mut c = nested_for_in_loop();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
