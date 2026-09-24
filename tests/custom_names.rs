//! `#[corot(name = …, effect = …)]` overrides generated enum names.

#![cfg(not(feature = "serde"))]

use corot_rs::corot;

#[corot(name = GoSanctuary, effect = GoSanctuaryEffect)]
async fn go_sanctuary_corot() {
    let n: i32 = ().await;
    println!("sanctuary {n}");
}

#[corot(name = Leaf)]
async fn leaf_corot() {
    let n: i32 = ().await;
    println!("leaf {n}");
}

#[corot(name = Root, effect = RootEffect)]
async fn root_corot() {
    let _: () = corot_rs::call::<Leaf>(leaf_corot()).await;
    let m: i32 = ().await;
    println!("root {m}");
}

#[test]
fn test_custom_name_and_effect() {
    // Must be `GoSanctuary`, not `GoSanctuaryCorotCoroutine`.
    let mut c: GoSanctuary = go_sanctuary_corot();
    assert!(matches!(c.step(), corot_rs::Step::Pending));
    match c.pending_slot().unwrap() {
        GoSanctuaryPendingSlot::N(s) => s.set(7),
    }
    assert!(matches!(c.step(), corot_rs::Step::Ready(())));
}

#[test]
fn test_name_only_defaults_effect_to_name_effect() {
    // `name = Leaf` ⇒ effect enum `LeafEffect` (required by nested `call::<Leaf>`).
    let mut c: Root = root_corot();
    let _leaf_ctor: fn() -> Leaf = leaf_corot;

    assert!(matches!(c.step(), corot_rs::Step::Pending));
    // Nested child slot: drill into `Leaf`, then settle its await.
    match c.pending_slot().unwrap() {
        RootPendingSlot::Unit0(leaf) => match leaf.pending_slot().unwrap() {
            LeafPendingSlot::N(s) => s.set(1),
        },
        RootPendingSlot::M(_) => panic!("expected nested leaf"),
    }
    assert!(matches!(c.step(), corot_rs::Step::Pending));
    match c.pending_slot().unwrap() {
        RootPendingSlot::M(s) => s.set(2),
        RootPendingSlot::Unit0(_) => panic!("expected own await"),
    }
    assert!(matches!(c.step(), corot_rs::Step::Ready(())));
}

// Force-link the overridden effect enums so a wrong default name fails to compile.
fn _effect_types(_: GoSanctuaryEffect, _: LeafEffect, _: RootEffect) {}
