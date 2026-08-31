#![cfg(feature = "serde")]

//! Richer serde demo: checkpoint mid-flight, then rehydrate skipped locals.

use corot_rs::{corot, SkipSerde};
use std::task::Poll;

/// Persisted identity — *is* serializable.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct UserId(u64);

/// Live DB/socket-style handle — deliberately **not** Serialize/Deserialize.
#[derive(Debug)]
struct LiveDb {
    fd: i32,
    label: &'static str,
}

impl LiveDb {
    fn connect(user: u64) -> Self {
        println!("  [LiveDb] connect user={user}");
        Self {
            fd: 1000 + user as i32,
            label: "live",
        }
    }

    fn query(&self, sql: &str) -> i32 {
        println!("  [LiveDb fd={}] query: {sql}", self.fd);
        self.fd
    }
}

fn scale(rows: i32) -> f64 {
    rows as f64 * 0.5
}

#[corot]
async fn checkout() {
    let user: UserId = UserId(42);
    let db: SkipSerde<LiveDb> = SkipSerde::new(LiveDb::connect(user.0));

    println!("phase1 user={:?} db={:?}", user, *db);

    let rows: i32 = db.query("select count(*)").await;
    println!("phase2 rows={rows} db.fd={}", db.fd);

    let total: f64 = scale(rows).await;
    println!(
        "phase3 total={total} user={:?} db={} (fd={})",
        user, db.label, db.fd
    );
}

fn dump(label: &str, c: &CheckoutCoroutine) {
    let json = serde_json::to_string_pretty(c).expect("serialize");
    println!("--- {label} ---\n{json}\n");
}

#[test]
fn test_with_serde() {
    println!("=== run A: step until first wait, checkpoint ===\n");
    let mut a = checkout();
    assert!(matches!(a.step(), Ok(Poll::Pending)));
    dump("after first step (WaitingRows)", &a);

    println!("=== restore into B from JSON (db skipped → needs rehydration) ===\n");
    let json = serde_json::to_string(&a).unwrap();
    let mut b: CheckoutCoroutine = serde_json::from_str(&json).unwrap();
    dump("freshly deserialized B", &b);

    b.settle_wait(&10);
    assert!(matches!(
        b.step(),
        Err(CheckoutCoroutineRehydration::NeedsRehydrationDb { .. })
    ));
    println!("step correctly returned NeedsRehydrationDb\n");

    let user_id = b.get_user().expect("user present in this state").0;
    match b.rehydrate() {
        CheckoutCoroutineRehydration::Ok => panic!("expected needs rehydration"),
        CheckoutCoroutineRehydration::NeedsRehydrationDb { db } => {
            println!("rehydrating db for user={user_id}");
            db.set(LiveDb::connect(user_id));
        }
    }

    assert!(matches!(b.rehydrate(), CheckoutCoroutineRehydration::Ok));
    assert!(matches!(b.step(), Ok(Poll::Pending))); // → WaitingTotal
    dump("B waiting for total", &b);

    b.settle_wait(&99.5);
    assert!(matches!(b.step(), Ok(Poll::Ready(()))));
    dump("B finished", &b);
}
