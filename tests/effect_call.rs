//! External async calls surface as `Step::Effect(CallFoo(…))` so the host
//! can invoke them, then settle via `pending_slot()` with the return value.

#![cfg(not(feature = "serde"))]

use corot_rs::corot;

async fn send_message(id: i32) -> i32 {
    id * 10
}

async fn fetch_user(id: i32) -> i32 {
    id + 100
}

async fn ping(_unit: ()) -> i32 {
    1
}

async fn pair(xy: (i32, bool)) -> i32 {
    if xy.1 { xy.0 } else { -xy.0 }
}

#[corot]
async fn chat() {
    println!("chat: before send");
    let reply: i32 = send_message(1).await;
    println!("chat: reply={reply}");
    let n: i32 = fetch_user(7).await;
    println!("chat: user={n}");
}

#[corot]
async fn with_local(user_id: i32) {
    let reply: i32 = send_message(user_id).await;
    println!("with_local: {reply}");
}

#[corot]
async fn unit_and_tuple_args() {
    let a: i32 = ping(()).await;
    let b: i32 = pair((3, true)).await;
    println!("unit_and_tuple: {a},{b}");
}

#[test]
fn test_effect_call() {
    let mut c = chat();

    assert!(matches!(
        c.step(),
        corot_rs::Step::Effect(ChatCoroutineEffect::CallSendMessage(1))
    ));
    // Host performs send_message(1) itself, then settles via the typed slot.
    let _ = send_message;
    match c.pending_slot().unwrap() {
        ChatCoroutinePendingSlot::Reply(s) | ChatCoroutinePendingSlot::N(s) => s.set(10),
    }

    assert!(matches!(
        c.step(),
        corot_rs::Step::Effect(ChatCoroutineEffect::CallFetchUser(7))
    ));
    let _ = fetch_user;
    match c.pending_slot().unwrap() {
        ChatCoroutinePendingSlot::Reply(s) | ChatCoroutinePendingSlot::N(s) => s.set(107),
    }

    assert!(matches!(c.step(), corot_rs::Step::Ready(())));
}

#[test]
fn test_effect_call_captured_arg() {
    let mut c = with_local(42);
    assert!(matches!(
        c.step(),
        corot_rs::Step::Effect(WithLocalCoroutineEffect::CallSendMessage(42))
    ));
    match c.pending_slot().unwrap() {
        WithLocalCoroutinePendingSlot::Reply(s) => s.set(420),
    }
    assert!(matches!(c.step(), corot_rs::Step::Ready(())));
}

#[test]
fn test_effect_call_unit_and_tuple_args() {
    let mut c = unit_and_tuple_args();

    assert!(matches!(
        c.step(),
        corot_rs::Step::Effect(UnitAndTupleArgsCoroutineEffect::CallPing(()))
    ));
    let _ = ping;
    match c.pending_slot().unwrap() {
        UnitAndTupleArgsCoroutinePendingSlot::A(s)
        | UnitAndTupleArgsCoroutinePendingSlot::B(s) => s.set(1),
    }

    assert!(matches!(
        c.step(),
        corot_rs::Step::Effect(
            UnitAndTupleArgsCoroutineEffect::CallPair((3, true))
        )
    ));
    let _ = pair;
    match c.pending_slot().unwrap() {
        UnitAndTupleArgsCoroutinePendingSlot::A(s)
        | UnitAndTupleArgsCoroutinePendingSlot::B(s) => s.set(3),
    }

    assert!(matches!(c.step(), corot_rs::Step::Ready(())));
}
