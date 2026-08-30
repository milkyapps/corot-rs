use corot_macros::corot;
use std::task::Poll;

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
    // calc before await (`pre_b(a)`), method call after await (`.print()`)
    let b: f64 = pre_b(a).await.print();
    println!("3 {b}");
}

fn main() {
    let mut c = f();

    assert!(matches!(c.step(), Poll::Pending)); // prints 1 → WaitingA
    c.settle_wait(&2);

    assert!(matches!(c.step(), Poll::Pending)); // prints 2 2, eval (2.0+1.5) → WaitingB
    c.settle_wait(&3.14);

    assert!(matches!(c.step(), Poll::Ready(()))); // print 3.14, prints 3 3.14 → Finished
    assert!(matches!(c.step(), Poll::Ready(())));
}
