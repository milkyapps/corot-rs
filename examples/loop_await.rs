//! Simple `loop` with one await inside (and `break`).
//!
//! Run: `cargo run --example loop_await`

use corot_macros::corot;
use std::task::Poll;

#[corot]
#[allow(unused_mut)]
async fn count_loop() {
    let mut sum: i32 = 0;
    println!("start");
    loop {
        println!("sum={sum}");
        let n: i32 = ().await;
        sum += n;
        println!("added n={n}, sum={sum}");
        if sum >= 10 {
            break;
        }
    }
    println!("done");
}

fn main() {
    let mut c = count_loop();

    // iteration 1
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3);

    // iteration 2
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&4);

    // iteration 3 → sum becomes 10 → break → done
    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3);

    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
