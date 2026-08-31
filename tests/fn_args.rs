//! `#[corot]` async fns with typed arguments.
//!
//! Args are stored on `NotStarted` and captured across awaits like locals.

use corot_rs::corot;


#[corot]
async fn greet(name: String) {
    println!("greet: before {name}");
    let n: i32 = ().await;
    println!("greet: {name} n={n}");
}

#[corot]
async fn accumulate(mut sum: i32, add: i32) {
    println!("acc: start sum={sum} add={add}");
    let x: i32 = ().await;
    sum += x + add;
    println!("acc: after first sum={sum}");
    let y: i32 = ().await;
    sum += y;
    println!("acc: done sum={sum}");
}

#[corot]
async fn sync_only(flag: bool) {
    if flag {
        println!("sync: true");
    } else {
        println!("sync: false");
    }
}

#[corot]
async fn leaf_with_arg(v: i32) {
    println!("leaf: v={v}");
    let n: i32 = ().await;
    println!("leaf: v+n={}", v + n);
}

#[corot]
async fn root_calls_leaf(seed: i32) {
    println!("root: seed={seed}");
    let _: () = corot_rs::call::<LeafWithArgCoroutine>(leaf_with_arg(seed + 1)).await;
    println!("root: done");
}

#[test]
fn test_fn_args() {
    println!("=== greet ===");
    let mut c = greet("Ada".into());
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&7i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== accumulate ===");
    let mut c = accumulate(10, 3);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&5i32); // sum = 10 + 5 + 3 = 18
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32); // sum = 20
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== sync only ===");
    let mut c = sync_only(true);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== compose with args ===");
    let mut c = root_calls_leaf(40);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32); // leaf: 41 + 2
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
