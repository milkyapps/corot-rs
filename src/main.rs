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
    let b: f64 = pre_b(a).await.print();
    println!("3 {b}");
}

fn main() {
    let mut c = f();

    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&2);

    assert!(matches!(c.step(), Ok(Poll::Pending)));
    c.settle_wait(&3.14);

    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
    assert!(matches!(c.step(), Ok(Poll::Ready(()))));
}
