//! `if let` and `let…else` with await.
//!
//! Scrutinee types use `corot_rs::val::<T>(…)` (or literal patterns like `Some(0)`).

use corot_rs::corot;


// --- if let: await on scrutinee ---

#[corot]
async fn if_let_scrutinee() {
    println!("scrut: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        println!("scrut: some {x}");
    } else {
        println!("scrut: none");
    }
    println!("scrut: after");
}

// --- if let: await in then ---

#[corot]
async fn if_let_then() {
    let opt: Option<i32> = Some(3);
    println!("then: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(opt) {
        println!("then: got x={x}, awaiting");
        let n: i32 = ().await;
        println!("then: x={x} n={n}");
    } else {
        println!("then: none");
    }
    println!("then: after");
}

// --- if let: await in else ---

#[corot]
async fn if_let_else() {
    let opt: Option<i32> = None;
    println!("else: before");
    if let Some(x) = corot_rs::val::<Option<i32>>(opt) {
        println!("else: unreachable {x}");
    } else {
        println!("else: none path, awaiting");
        let n: i32 = ().await;
        println!("else: n={n}");
    }
    println!("else: after");
}

// --- let…else: await in initializer ---

#[corot]
async fn let_else_init() {
    println!("init: before");
    let Some(x) = corot_rs::val::<Option<i32>>(().await) else {
        println!("init: else diverged");
        return;
    };
    println!("init: x={x}");
    println!("init: after");
}

// --- let…else: await in else block ---

#[corot]
async fn let_else_diverge() {
    let opt: Option<i32> = None;
    println!("diverge: before");
    let Some(x) = corot_rs::val::<Option<i32>>(opt) else {
        println!("diverge: in else, awaiting");
        let n: i32 = ().await;
        println!("diverge: n={n}, returning");
        return;
    };
    println!("diverge: unreachable x={x}");
}

#[corot]
async fn let_else_success() {
    let opt: Option<i32> = Some(9);
    let Some(x) = corot_rs::val::<Option<i32>>(opt) else {
        let n: i32 = ().await;
        println!("unreachable n={n}");
        return;
    };
    println!("success: x={x}");
}

// --- else if let: await on scrutinee ---

#[corot]
async fn else_if_let_scrutinee() {
    let a: bool = false;
    println!("eils: before");
    if a {
        println!("eils: first");
    } else if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        println!("eils: some {x}");
    } else {
        println!("eils: none");
    }
    println!("eils: after");
}

#[corot]
async fn else_if_let_scrutinee_skipped() {
    let a: bool = true;
    println!("eils-skip: before");
    if a {
        println!("eils-skip: first (no suspend)");
    } else if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        println!("eils-skip: unreachable {x}");
    } else {
        println!("eils-skip: unreachable else");
    }
    println!("eils-skip: after");
}

#[corot]
async fn else_if_let_scrutinee_nested_skip() {
    let a: bool = false;
    let b: bool = false;
    println!("eils-nest: before");
    if a {
        println!("eils-nest: first");
    } else if b {
        println!("eils-nest: second");
    } else if let Some(x) = corot_rs::val::<Option<i32>>(().await) {
        println!("eils-nest: some {x}");
    } else {
        println!("eils-nest: none");
    }
    println!("eils-nest: after");
}

#[test]
fn test_if_let_await() {
    println!("=== if let scrutinee ===");
    let mut c = if_let_scrutinee();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Some(7));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    let mut c = if_let_scrutinee();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== if let then ===");
    let mut c = if_let_then();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&11);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== if let else ===");
    let mut c = if_let_else();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&22);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== let else init (Some) ===");
    let mut c = let_else_init();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Some(5));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== let else init (None → return) ===");
    let mut c = let_else_init();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== let else diverge await ===");
    let mut c = let_else_diverge();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&33);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== let else success (no suspend) ===");
    let mut c = let_else_success();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!();
    println!("=== else if let scrutinee (Some) ===");
    let mut c = else_if_let_scrutinee();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Some(7));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!();
    println!("=== else if let scrutinee (None) ===");
    let mut c = else_if_let_scrutinee();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&None::<i32>);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!();
    println!("=== else if let scrutinee (skipped) ===");
    let mut c = else_if_let_scrutinee_skipped();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!();
    println!("=== else if let scrutinee (nested skip → Some) ===");
    let mut c = else_if_let_scrutinee_nested_skip();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&Some(4));
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
