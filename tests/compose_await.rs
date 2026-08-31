//! Compose `#[corot]` functions via `call::<Child>(…).await`.
//!
//! The child is the generated coroutine enum (not a Rust `Future`). The parent
//! drives `child.step()` / forwards `settle_wait` until `corot_rs::Step::Ready`.

use corot_rs::corot;


#[corot]
async fn leaf() {
    println!("leaf: before");
    let n: i32 = ().await;
    println!("leaf: n={n}");
}

#[corot]
async fn mid() {
    println!("mid: before leaf");
    let _: () = corot_rs::call::<LeafCoroutine>(leaf()).await;
    println!("mid: after leaf");
    let m: i32 = ().await;
    println!("mid: own await m={m}");
}

#[corot]
async fn root() {
    println!("root: start");
    let _: () = corot_rs::call::<MidCoroutine>(mid()).await;
    println!("root: after mid");
    let _: () = corot_rs::call::<LeafCoroutine>(leaf()).await;
    println!("root: done");
}

#[test]
fn test_compose_await() {
    println!("=== compose root → mid → leaf ===");
    let mut c = root();

    // Enter mid → leaf → leaf's await
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&10i32); // leaf
    // mid's own await
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&20i32); // mid
    // root's second leaf
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&30i32); // leaf again
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
