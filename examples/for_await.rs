//! `for` with await on the iterable and await in the body.
//!
//! Run: `cargo run -p corot-rs --example for_await`

use corot_macros::corot;
use std::task::Poll;

#[corot]
async fn for_range() {
    println!("start");
    // await in the `in` expression, then await again inside the body
    for i in (0..3).await {
        println!("i={i}");
        let n: i32 = ().await;
        println!("i={i} n={n}");
    }
    println!("done");
}

fn main() {
    let mut c = for_range();

    // 1) wait for the iterable
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&(0..3));

    // 2) three body iterations
    for expected_i in 0..3 {
        assert!(matches!(c.step(), Ok(Poll::Pending)));
        c.settle_wait(&(expected_i * 10));
    }

    // LoopHead sees end of range → AfterLoop → Finished
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
