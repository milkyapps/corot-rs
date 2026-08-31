//! Richer patterns on suspending `match` arms and `for` loops:
//! struct, custom tuple-struct, and slice/array.

#![allow(unused_mut, unreachable_code, dead_code)]

use corot_rs::corot;

struct Point {
    x: i32,
    y: i32,
}

struct Pair(i32, i32);

#[corot]
async fn match_struct_arm() {
    let p = Point { x: 1, y: 2 };
    match corot_rs::val_fields::<Point, (i32, i32)>(p) {
        Point { x, y } => {
            let n: i32 = ().await;
            println!("struct: x={x} y={y} n={n}");
        }
    }
}

#[corot]
async fn match_tuple_struct_arm() {
    let p = Pair(3, 4);
    match corot_rs::val_fields::<Pair, (i32, i32)>(p) {
        Pair(a, b) => {
            let n: i32 = ().await;
            println!("tuple-struct: a={a} b={b} n={n}");
        }
    }
}

#[corot]
async fn match_slice_arm() {
    let xs: [i32; 2] = [5, 6];
    match corot_rs::val::<[i32; 2]>(xs) {
        [a, b] => {
            let n: i32 = ().await;
            println!("slice: a={a} b={b} n={n}");
        }
    }
}

#[corot]
async fn match_struct_at_hint() {
    let p = Point { x: 7, y: 8 };
    // `@` literal hints type fields without `val_fields`.
    match corot_rs::val::<Point>(p) {
        Point {
            x: x @ 7,
            y: y @ 8,
        } => {
            let n: i32 = ().await;
            println!("at-hint: x={x} y={y} n={n}");
        }
        _ => println!("at-hint: other"),
    }
}

#[corot]
async fn for_struct_items() {
    let pts = vec![Point { x: 1, y: 10 }, Point { x: 2, y: 20 }];
    for Point { x, y } in corot_rs::iter_fields::<Vec<Point>, (i32, i32)>(pts) {
        let n: i32 = ().await;
        println!("for-struct: x={x} y={y} n={n}");
    }
}

#[corot]
async fn for_tuple_struct_items() {
    let pairs = vec![Pair(1, 2), Pair(3, 4)];
    for Pair(a, b) in corot_rs::iter_fields::<Vec<Pair>, (i32, i32)>(pairs) {
        let n: i32 = ().await;
        println!("for-pair: a={a} b={b} n={n}");
    }
}

#[corot]
async fn for_array_items() {
    let rows = vec![[1, 2], [3, 4]];
    for [a, b] in corot_rs::iter::<Vec<[i32; 2]>>(rows) {
        let n: i32 = ().await;
        println!("for-arr: a={a} b={b} n={n}");
    }
}

#[corot]
async fn match_struct_range_at() {
    let p = Point { x: 3, y: 4 };
    match corot_rs::val::<Point>(p) {
        Point {
            x: x @ 0..=10,
            y: y @ 0..=10,
        } => {
            let n: i32 = ().await;
            println!("range-at: x={x} y={y} n={n}");
        }
        _ => println!("range-at: other"),
    }
}

#[test]
fn test_richer_patterns() {
    println!("=== match struct ===");
    let mut c = match_struct_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&11i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match tuple-struct ===");
    let mut c = match_tuple_struct_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&22i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match slice ===");
    let mut c = match_slice_arm();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&33i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match @ hint ===");
    let mut c = match_struct_at_hint();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&44i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== for struct ===");
    let mut c = for_struct_items();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== for tuple-struct ===");
    let mut c = for_tuple_struct_items();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== for array ===");
    let mut c = for_array_items();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&1i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&2i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));

    println!("=== match range @ ===");
    let mut c = match_struct_range_at();
    assert!(matches!(c.step(), Ok(corot_rs::Step::Pending)));
    c.settle_wait(&55i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
