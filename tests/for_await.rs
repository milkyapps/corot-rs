//! `for` over ranges or any `IntoIterator` via `corot_rs::iter::<I>(…)`.

use corot_rs::corot;


// --- range sugar (still supported) ---

#[corot]
async fn for_range() {
    println!("range: start");
    for i in (0..3).await {
        println!("range: i={i}");
        let n: i32 = ().await;
        println!("range: i={i} n={n}");
    }
    println!("range: done");
}

// --- arbitrary IntoIterator (Vec) with await on the iterable + body ---

#[corot]
async fn for_vec() {
    println!("vec: start");
    for i in corot_rs::iter::<Vec<i32>>(().await) {
        println!("vec: i={i}");
        let n: i32 = ().await;
        println!("vec: i={i} n={n}");
    }
    println!("vec: done");
}

// --- sync iterable, await only in the body ---

#[corot]
async fn for_vec_sync() {
    println!("sync: start");
    for i in corot_rs::iter::<Vec<i32>>(vec![7, 8]) {
        let n: i32 = ().await;
        println!("sync: i={i} n={n}");
    }
    println!("sync: done");
}

fn run_range() {
    println!("=== range ===");
    let mut c = for_range();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&(0..3));
    for expected_i in 0..3 {
        assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
        c.settle_wait(&(expected_i * 10));
    }
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();
}

fn run_vec() {
    println!("=== vec (await iterable) ===");
    let mut c = for_vec();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&vec![1, 2, 3]);
    for expected_i in 1..=3 {
        assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
        c.settle_wait(&(expected_i * 10));
    }
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();
}

fn run_vec_sync() {
    println!("=== vec (sync iterable) ===");
    let mut c = for_vec_sync();
    for expected_i in [7, 8] {
        assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
        c.settle_wait(&(expected_i * 10));
    }
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();
}

#[test]
fn test_for_await() {
    run_range();
    run_vec();
    run_vec_sync();
}
