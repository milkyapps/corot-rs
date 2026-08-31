//! External async calls surface as `Step::Effect(CallFoo(…))` so the host
//! can invoke them, then `settle_wait` with the return value.

use corot_rs::corot;

async fn send_message(id: i32) -> i32 {
    id * 10
}

async fn fetch_user(id: i32) -> i32 {
    id + 100
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

#[test]
fn test_effect_call() {
    let mut c = chat();

    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(ChatCoroutineEffect::CallSendMessage(1)))
    ));
    // Host performs send_message(1) itself, then settles the return value.
    let _ = send_message;
    c.settle_wait(&10i32);

    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(ChatCoroutineEffect::CallFetchUser(7)))
    ));
    let _ = fetch_user;
    c.settle_wait(&107i32);

    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}

#[test]
fn test_effect_call_captured_arg() {
    let mut c = with_local(42);
    assert!(matches!(
        c.step(),
        Ok(corot_rs::Step::Effect(WithLocalCoroutineEffect::CallSendMessage(42)))
    ));
    c.settle_wait(&420i32);
    assert!(matches!(c.step(), Ok(corot_rs::Step::Ready(()))));
}
