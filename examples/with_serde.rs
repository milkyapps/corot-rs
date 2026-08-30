//! Richer serde demo: checkpoint a coroutine mid-flight when some locals
//! cannot be serialized.
//!
//! What we learn:
//! - Proc macros cannot detect `Serialize` / `Deserialize` impls.
//! - Wrap opaque runtime values in `SkipSerde<T>` so the macro emits
//!   `#[serde(skip)]` on those capture fields.
//! - Serializable locals round-trip through JSON; skipped ones come back as
//!   `Default`, so you rehydrate them after load (reconnect, reopen, …).
//!
//! Run: `cargo run --example with_serde --features serde`

use corot_rs::{corot, SkipSerde};
use std::task::Poll;

/// Persisted identity — *is* serializable.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct UserId(u64);

/// Live DB/socket-style handle — deliberately **not** Serialize/Deserialize.
#[derive(Debug)]
struct LiveDb {
    /// Pretend OS resource; meaningless after a process restart.
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

impl Default for LiveDb {
    /// Placeholder after `#[serde(skip)]` deserialize — not a real connection.
    fn default() -> Self {
        Self {
            fd: -1,
            label: "deserialized-empty",
        }
    }
}

fn scale(rows: i32) -> f64 {
    rows as f64 * 0.5
}

#[corot]
async fn checkout() {
    let user: UserId = UserId(42);
    let db: SkipSerde<LiveDb> = SkipSerde(LiveDb::connect(user.0));

    println!("phase1 user={:?} db={:?}", user, db.0);

    // Wait for an external “rows fetched” value.
    let rows: i32 = db.0.query("select count(*)").await;
    println!("phase2 rows={rows} db.fd={}", db.0.fd);

    // Wait for a priced total; pre-await work runs `scale(rows)`.
    let total: f64 = scale(rows).await;
    println!(
        "phase3 total={total} user={:?} db={} (fd={})",
        user, db.0.label, db.0.fd
    );
}

fn dump(label: &str, c: &CheckoutCoroutine) {
    let json = serde_json::to_string_pretty(c).expect("serialize");
    println!("--- {label} ---\n{json}\n");
}

fn main() {
    println!("=== run A: step until first wait, checkpoint ===\n");
    let mut a = checkout();
    assert!(matches!(a.step(), Poll::Pending)); // → WaitingRows (db skipped in JSON)
    dump("after first step (WaitingRows)", &a);

    println!("=== restore into B from JSON (db handle is gone) ===\n");
    let json = serde_json::to_string(&a).unwrap();
    let mut b: CheckoutCoroutine = serde_json::from_str(&json).unwrap();
    dump("freshly deserialized B", &b);

    // Rehydrate the opaque resource using serializable state we kept.
    // (In a real system: reopen pool connection from UserId / config.)
    if let CheckoutCoroutine::WaitingRows { user, db, .. } = &mut b {
        println!("before rehydrate: db.label={} fd={}\n", db.0.label, db.0.fd);
        *db = SkipSerde(LiveDb::connect(user.0));
        println!("after rehydrate: db={:?}\n", db.0);
    }

    b.settle_wait(&10); // rows
    assert!(matches!(b.step(), Poll::Pending)); // → WaitingTotal
    dump(
        "B waiting for total (user+rows serializable; db still skipped)",
        &b,
    );

    b.settle_wait(&99.5);
    assert!(matches!(b.step(), Poll::Ready(())));
    dump("B finished", &b);
}
