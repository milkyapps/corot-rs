//! Multiple match arms that each await (exclusive siblings → shared AfterMatch).

#![allow(unused_mut, unreachable_code)]

use corot_rs::corot;

#[corot]
async fn multi_arm_first() {
    let k: i32 = 0;
    match k {
        0 => {
            let a: i32 = ().await;
            println!("multi first a={a}");
        }
        1 => {
            let b: i32 = ().await;
            println!("multi first b={b}");
        }
        _ => println!("multi first other"),
    }
}

#[corot]
async fn multi_arm_second() {
    let k: i32 = 1;
    match k {
        0 => {
            let a: i32 = ().await;
            println!("multi second a={a}");
        }
        1 => {
            let b: i32 = ().await;
            println!("multi second b={b}");
        }
        _ => println!("multi second other"),
    }
}

#[corot]
async fn multi_arm_sync() {
    let k: i32 = 9;
    match k {
        0 => {
            let a: i32 = ().await;
            println!("unreachable a={a}");
        }
        1 => {
            let b: i32 = ().await;
            println!("unreachable b={b}");
        }
        _ => println!("multi sync other"),
    }
}

#[corot]
async fn multi_arm_expr() {
    let k: i32 = 1;
    let v: i32 = match k {
        0 => {
            let a: i32 = ().await;
            a + 1
        }
        1 => {
            let b: i32 = ().await;
            b + 2
        }
        _ => 0,
    };
    println!("multi expr v={v}");
}

#[corot]
async fn multi_arm_then_await() {
    let k: i32 = 0;
    match k {
        0 => {
            let a: i32 = ().await;
            println!("then-await a={a}");
        }
        _ => {
            let b: i32 = ().await;
            println!("then-await b={b}");
        }
    }
    let z: i32 = ().await;
    println!("then-await z={z}");
}

#[corot]
async fn multi_arm_multi_body() {
    let k: i32 = 1;
    match k {
        0 => {
            let a: i32 = ().await;
            let a2: i32 = ().await;
            println!("body0 a={a} a2={a2}");
        }
        1 => {
            let b: i32 = ().await;
            println!("body1 b={b}");
        }
        _ => {}
    }
}

#[test]
fn test_multi_match_arm_await() {
    println!("=== multi arm first ===");
    let mut c = multi_arm_first();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&10i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== multi arm second ===");
    let mut c = multi_arm_second();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&20i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== multi arm sync ===");
    let mut c = multi_arm_sync();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== multi arm expr ===");
    let mut c = multi_arm_expr();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&5i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== multi arm then await ===");
    let mut c = multi_arm_then_await();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== multi arm multi body ===");
    let mut c = multi_arm_multi_body();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
