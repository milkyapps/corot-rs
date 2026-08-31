//! Mid-expression awaits outside bare typed `let x = ….await`:
//! `foo(val::<T>(a()).await, val::<U>(b()).await)`.

#![allow(unused_mut, unreachable_code, dead_code)]

use corot_rs::corot;

fn add(a: i32, b: i32) -> i32 {
    a + b
}

async fn left() -> i32 {
    3
}

async fn right() -> i32 {
    4
}

#[corot]
async fn call_two_args() {
    let v: i32 = add(
        corot_rs::val::<i32>(left()).await,
        corot_rs::val::<i32>(right()).await,
    );
    println!("call_two_args v={v}");
}

#[corot]
async fn call_two_args_stmt() {
    let _ = add(
        corot_rs::val::<i32>(left()).await,
        corot_rs::val::<i32>(right()).await,
    );
}

#[corot]
async fn method_two_awaits() {
    let v: i32 = corot_rs::val::<i32>(left())
        .await
        .saturating_add(corot_rs::val::<i32>(right()).await);
    println!("method_two_awaits v={v}");
}

#[corot]
async fn binary_two_awaits() {
    let v: i32 = corot_rs::val::<i32>(left()).await + corot_rs::val::<i32>(right()).await;
    println!("binary_two_awaits v={v}");
}

#[corot]
async fn tuple_awaits() {
    let pair: (i32, i32) = (
        corot_rs::val::<i32>(left()).await,
        corot_rs::val::<i32>(right()).await,
    );
    println!("tuple_awaits {pair:?}");
}

#[corot]
async fn single_mid_keeps_let_ty() {
    // Single mid-await still uses the outer let type as settle type.
    let v: i32 = add(left().await, 1);
    println!("single_mid v={v}");
}

#[corot]
async fn mid_then_plain() {
    let v: i32 = add(
        corot_rs::val::<i32>(left()).await,
        corot_rs::val::<i32>(right()).await,
    );
    let z: i32 = ().await;
    println!("mid_then_plain v+z={}", v + z);
}

#[test]
fn test_mid_expr_await() {
    println!("=== call two args ===");
    let mut c = call_two_args();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(CallTwoArgsCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&3i32);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(CallTwoArgsCoroutineEffect::CallRight()))
    ));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== call two args stmt ===");
    let mut c = call_two_args_stmt();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(CallTwoArgsStmtCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&3i32);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(CallTwoArgsStmtCoroutineEffect::CallRight()))
    ));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== method two awaits ===");
    let mut c = method_two_awaits();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(MethodTwoAwaitsCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&3i32);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(MethodTwoAwaitsCoroutineEffect::CallRight()))
    ));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== binary two awaits ===");
    let mut c = binary_two_awaits();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(BinaryTwoAwaitsCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&3i32);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(BinaryTwoAwaitsCoroutineEffect::CallRight()))
    ));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== tuple awaits ===");
    let mut c = tuple_awaits();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(TupleAwaitsCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&3i32);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(TupleAwaitsCoroutineEffect::CallRight()))
    ));
    c.settle_wait(&4i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== single mid ===");
    let mut c = single_mid_keeps_let_ty();
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(SingleMidKeepsLetTyCoroutineEffect::CallLeft()))
    ));
    c.settle_wait(&10i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== mid then plain ===");
    let mut c = mid_then_plain();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Effect(_))));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Effect(_))));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&0i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
