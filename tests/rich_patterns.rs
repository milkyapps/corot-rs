//! Richer patterns on suspending `match` arms and `for` loops.

use corot_rs::corot;


#[corot]
async fn match_some_arm() {
    let opt: Option<i32> = Some(3);
    println!("some: before");
    match corot_rs::val::<Option<i32>>(opt) {
        None => println!("some: none"),
        Some(x) => {
            println!("some: x={x}");
            let n: i32 = ().await;
            println!("some: x={x} n={n}");
        }
    }
    println!("some: after");
}

#[corot]
async fn match_tuple_arm() {
    let pair: (i32, bool) = (4, true);
    println!("tup: before");
    match corot_rs::val::<(i32, bool)>(pair) {
        (0, _) => println!("tup: zero"),
        (a, true) => {
            println!("tup: a={a}");
            let n: i32 = ().await;
            println!("tup: a={a} n={n}");
        }
        (a, false) => println!("tup: false a={a}"),
    }
    println!("tup: after");
}

#[corot]
async fn match_ok_arm() {
    let r: Result<i32, &'static str> = Ok(9);
    println!("ok: before");
    match corot_rs::val::<Result<i32, &'static str>>(r) {
        Err(e) => println!("ok: err {e}"),
        Ok(v) => {
            let n: i32 = ().await;
            println!("ok: v={v} n={n}");
        }
    }
    println!("ok: after");
}

#[corot]
async fn for_tuple_items() {
    println!("for-tup: start");
    for (a, b) in corot_rs::iter::<Vec<(i32, i32)>>(vec![(1, 10), (2, 20)]) {
        let n: i32 = ().await;
        println!("for-tup: a={a} b={b} n={n}");
    }
    println!("for-tup: done");
}

#[corot]
async fn for_option_items() {
    println!("for-opt: start");
    for Some(x) in corot_rs::iter::<Vec<Option<i32>>>(vec![Some(5), None, Some(7)]) {
        let n: i32 = ().await;
        println!("for-opt: x={x} n={n}");
    }
    println!("for-opt: done");
}

#[test]
fn test_rich_patterns() {
    println!("=== match Some(x) ===");
    let mut c = match_some_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&11i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== match (a, true) ===");
    let mut c = match_tuple_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&22i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== match Ok(v) ===");
    let mut c = match_ok_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&33i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== for (a, b) ===");
    let mut c = for_tuple_items();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&100i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&200i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
    println!();

    println!("=== for Some(x) (skips None) ===");
    let mut c = for_option_items();
    // first Some(5)
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    // None is skipped by pattern; next Some(7)
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
