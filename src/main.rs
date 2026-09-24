use corot_macros::corot;

trait Print {
    fn print(self) -> Self;
}

impl Print for f64 {
    fn print(self) -> Self {
        println!("print {self}");
        self
    }
}

fn pre_b(a: i32) -> f64 {
    a as f64 + 1.5
}

#[corot]
async fn f() {
    println!("1");
    let a: i32 = ().await;
    println!("2 {a}");
    let b: f64 = pre_b(a).await;
    let b = b.print();
    println!("3 {b}");
}

fn main() {
    // `step` is bare `Step` by default; with `serde` it is `Result<Step, Rehydration>`.
    macro_rules! assert_step {
        ($e:expr, $pat:pat) => {{
            let __v = $e;
            #[cfg(feature = "serde")]
            assert!(matches!(__v, Ok($pat)));
            #[cfg(not(feature = "serde"))]
            assert!(matches!(__v, $pat));
        }};
    }

    let mut c = f();

    assert_step!(c.step(), corot_rs::Step::Pending);
    match c.pending_slot().unwrap() {
        FCoroutinePendingSlot::A(s) => s.set(2),
        FCoroutinePendingSlot::B(_) => unreachable!("expected A"),
    }

    assert_step!(
        c.step(),
        corot_rs::Step::Effect(FCoroutineEffect::CallPreB(2))
    );
    match c.pending_slot().unwrap() {
        FCoroutinePendingSlot::A(_) => unreachable!("expected B"),
        FCoroutinePendingSlot::B(s) => s.set(pre_b(2)),
    }

    assert_step!(c.step(), corot_rs::Step::Ready(()));
    assert_step!(c.step(), corot_rs::Step::Ready(()));
}
