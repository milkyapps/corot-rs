//! Compose `#[corot]` functions via `call::<Child>(…).await`.
//!
//! The child is the generated coroutine enum (not a Rust `Future`). The parent
//! drives `child.step()` / forwards `settle_wait` until `Poll::Ready`.
//!
//! Run: `cargo run -p corot-rs --example compose_await`

use corot_macros::corot;
use std::task::Poll;

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

fn main() {
    println!("=== compose root → mid → leaf ===");
    let mut c = root();

    // Enter mid → leaf → leaf's await
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&10i32); // leaf
    // mid's own await
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&20i32); // mid
    // root's second leaf
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&30i32); // leaf again
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
