use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::{
    parse_macro_input, Expr, ExprPath, Ident, ItemFn, Lifetime, Lit, Local, LocalInit, Pat, PatIdent,
    PatType, Path, Stmt, Type,
};

/// Suspension points: typed `let` awaits (including `await?`); `if` / `if let` /
/// `let…else` / `loop` / `for` / `match` with a single await in a supported position.
///
/// - `let name: T = expr.await?` when the fn returns `Result<(), E>` (settle
///   `Result<T, E>`; `Err` finishes with `Poll::Ready(Err(...))`)
/// - general `expr?` in a `Result<(), E>` fn (rewritten to finish with `Err`)
/// - `try { … }` blocks (including with await / `await?`): desugared; block type
///   must be written as `let name: Result<T, E> = try { … }`
/// - `return` / `return <expr>` before or after an await (rewritten to finish the
///   coroutine with `Poll::Ready(...)`)
/// - `corot_rs::call::<ChildCoroutine>(child()).await` — drive another `#[corot]`
///   coroutine to completion (composition)
/// - `loop` / `while` / `while let` / `for`: optional label; `break` / `continue`
///   before or after the await (unlabeled or `'label`); unlabeled `break`/`continue`
///   inside nested sync loops stay native
/// - `let x: T = loop { …; break value }` / `break 'label value` (loop-as-expression)
/// - `if` / `if let`: condition/scrutinee, then, else, or else-if chain
/// - `let…else`: await in the initializer or in the `else` block
/// - `match`: scrutinee, one arm body, or one guard
/// - `for`: range literal or `iter::<I>(…)`, optional body await
/// - `while` / `while let`: await in the condition/scrutinee **or** the body (not both)
/// - labeled blocks: `let x: T = 'a: { …; break 'a value }` (await in the block)
///
/// Locals that live across an await must be type-annotated.
/// Scrutinee types for `if let` / `let…else` use pattern literals or
/// `corot_rs::val::<T>(…)`.
///
/// With the `serde` feature, wrap non-serializable captures in `SkipSerde<T>`
/// (from the `corot-rs` crate). The macro matches that type name and emits
/// `#[serde(skip)]` — it cannot detect `Serialize` bounds itself.
#[proc_macro_attribute]
pub fn corot(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    match expand_corot(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

struct AwaitPoint {
    /// Binding / variant stem, e.g. `a` → `WaitingA`.
    name: Ident,
    /// Temporary holding the settled await value (`__await_a`).
    tmp: Ident,
    /// Type provided by `settle_wait` (await output), or child `Output` when nested.
    wait_ty: Type,
    /// Expression evaluated before suspending (the await receiver/base).
    base: Expr,
    /// Statements before this await's statement/`if`.
    before: Vec<Stmt>,
    kind: SuspendKind,
    /// `let name: OkTy = ….await?` — settle `Result<OkTy, E>`, bind on `Ok`.
    try_ok: Option<(Ident, Type)>,
    /// `call::<ChildCoroutine>(…)` — drive another `#[corot]` enum.
    nested_child: Option<Type>,
}

struct PlainAwait {
    name: Ident,
    tmp: Ident,
    wait_ty: Type,
    base: Expr,
    after_resume: Vec<Stmt>,
    try_ok: Option<(Ident, Type)>,
    nested_child: Option<Type>,
}

enum SuspendKind {
    /// Top-level `let name: Ty = <expr with await>` (possibly with `await?` via `try_ok`).
    Plain {
        after_resume: Vec<Stmt>,
    },
    /// `if EXPR.await { then } else { else }` (EXPR.await is the whole condition)
    IfCondition {
        resume_cond: Expr,
        then_branch: syn::Block,
        else_branch: Option<Box<Expr>>,
    },
    /// Await only inside the then branch (`if` or `if let`).
    IfThen {
        cond: Expr,
        /// Bindings introduced by `if let` pattern (typed from scrutinee).
        pat_binds: Vec<Binding>,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        else_branch: Option<Box<Expr>>,
        /// Stmts after the `if` until the next await / end (run in `AfterIfN`).
        join_stmts: Vec<Stmt>,
    },
    /// Await only inside the else branch / else-if chain (`if` or `if let`).
    IfElse {
        cond: Expr,
        then_branch: syn::Block,
        else_suspend: ElseSuspend,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `if let PAT = EXPR.await { then } else { else }`
    IfLetScrutinee {
        pat: Pat,
        then_branch: syn::Block,
        else_branch: Option<Box<Expr>>,
    },
    /// `let PAT = EXPR else { … await … }` — await only in the else block.
    LetElseAwait {
        pat: Pat,
        init: Expr,
        pat_binds: Vec<Binding>,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `loop { before; let name: Ty = ….await; after; }` with optional label /
    /// `break` / `continue` (including `'label`).
    ///
    /// When used as `let bind: T = loop { …; break value }`, `break_bind` is set
    /// and `break`/`break value` assign that binding before joining.
    Loop {
        label: Option<Ident>,
        /// Present for loop-as-expression (`let name: Ty = loop { … }`).
        break_bind: Option<(Ident, Type)>,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `while COND { … }` / `while let PAT = EXPR { … }` with optional label.
    ///
    /// Await in the condition/scrutinee **or** in the body (not both).
    While {
        label: Option<Ident>,
        /// Sync condition (`bool` or `let PAT = EXPR`). `None` ⇒ condition is awaited.
        sync_cond: Option<Expr>,
        /// Pattern when awaiting a `while let` scrutinee (`sync_cond` is `None`).
        await_let_pat: Option<Pat>,
        /// Bindings from sync `while let` (live across a body await).
        pat_binds: Vec<Binding>,
        has_body_await: bool,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `for x in ITER { … }` with optional await on the iterable and/or body.
    ///
    /// `ITER` is a range literal (`0..3`, `(0..3).await`) or
    /// `corot_rs::iter::<I>(…)` / `iter::<I>(…)` where `I: IntoIterator`
    /// (the type of the `in` expression / settle value).
    For {
        label: Option<Ident>,
        item: Ident,
        /// Type of the `in` expression (`IntoIterator`), e.g. `Vec<i32>` or `Range<i32>`.
        into_ty: Type,
        /// `None` ⇒ iterable is awaited (`iter_await_base`); `Some` ⇒ sync expr.
        iter_expr: Option<Expr>,
        iter_await_base: Option<Expr>,
        has_body_await: bool,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `match EXPR.await { arms… }` — settle value is the scrutinee.
    MatchScrutinee {
        arms: Vec<syn::Arm>,
    },
    /// Await inside exactly one match arm body.
    MatchArm {
        scrutinee: Expr,
        scrut_ty: Type,
        pat_binds: Vec<Ident>,
        arms_before: Vec<syn::Arm>,
        sus_pat: Pat,
        sus_guard: Option<Box<Expr>>,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        arms_after: Vec<syn::Arm>,
        join_stmts: Vec<Stmt>,
    },
    /// Await in exactly one match guard (`if expr.await`, bool).
    /// Scrutinee must be `Clone` (stored across the guard await for fallthrough).
    MatchGuard {
        scrutinee: Expr,
        scrut_ty: Type,
        arms_before: Vec<syn::Arm>,
        sus_pat: Pat,
        sus_body: Box<Expr>,
        arms_after: Vec<syn::Arm>,
        join_stmts: Vec<Stmt>,
    },
    /// `'label: { …; let x: T = ….await; …; break 'label value }` as a `let` init
    /// or as a statement (unit value).
    LabeledBlock {
        label: Ident,
        /// Binding that receives the block value (`break 'label expr` / fallthrough).
        bind_name: Ident,
        bind_ty: Type,
        /// Statement form `'a: { … }` (no outer `let`); fallthrough is `()`.
        is_stmt: bool,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `let name: Result<T, E> = try { …; let x: T = ….await?; … }`
    ///
    /// `?` / `await?` Err paths assign `Err(…)` to `bind_name` and join; fallthrough
    /// wraps the trailing expression in `Ok`.
    TryBlock {
        bind_name: Ident,
        bind_ty: Type,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
}

/// Where the single await lives inside an `else` / `else if` chain.
enum ElseSuspend {
    /// `else { before; await; after }`
    FinalBlock {
        before_await: Vec<Stmt>,
    },
    /// `else if COND { before; await; after } else REST`
    ElseIfThen {
        cond: Expr,
        before_await: Vec<Stmt>,
        rest_else: Option<Box<Expr>>,
    },
    /// `else if COND { sync } else <rest with await>`
    ElseIfSkip {
        cond: Expr,
        then_branch: syn::Block,
        rest: Box<ElseSuspend>,
    },
    /// `else if EXPR.await { then } else REST` (bool settle)
    ElseIfCond {
        then_branch: syn::Block,
        rest_else: Option<Box<Expr>>,
    },
}

#[derive(Clone)]
struct Binding {
    name: Ident,
    ty: Type,
    mutable: bool,
}

fn expand_corot(input: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    if input.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &input.sig.fn_token,
            "#[corot] only supports async fn",
        ));
    }
    if !input.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.sig.inputs,
            "#[corot] basic version does not support function arguments yet",
        ));
    }

    let vis = &input.vis;
    let fn_name = &input.sig.ident;
    let enum_name = coroutine_name(fn_name);
    let (output_ty, err_ty) = parse_fn_output(&input.sig.output)?;
    let ready_ok = ready_ok_tokens(&output_ty);

    let (mut awaits, mut after_last) = split_awaits(&input.block.stmts, err_ty.as_ref())?;
    assign_join_stmts(&mut awaits, &mut after_last);

    let mut live: Vec<Binding> = Vec::new();
    let mut captures_at_await: Vec<Vec<Binding>> = Vec::new();
    let mut join_caps_at: Vec<Vec<Binding>> = Vec::new();

    for ap in &awaits {
        for stmt in &ap.before {
            if let Some(b) = typed_let_binding(stmt) {
                upsert_binding(&mut live, b);
            }
        }
        // Caps available on both then/else paths when entering AfterIf.
        join_caps_at.push(live.clone());

        match &ap.kind {
            SuspendKind::IfThen {
                pat_binds,
                before_await,
                ..
            } => {
                for b in pat_binds {
                    upsert_binding(&mut live, b.clone());
                }
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::IfElse {
                else_suspend,
                ..
            } => {
                for stmt in else_suspend_before_await(else_suspend) {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::Loop { before_await, .. } => {
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::While {
                pat_binds,
                before_await,
                has_body_await,
                ..
            } => {
                if *has_body_await {
                    for b in pat_binds {
                        upsert_binding(&mut live, b.clone());
                    }
                    for stmt in before_await {
                        if let Some(b) = typed_let_binding(stmt) {
                            upsert_binding(&mut live, b);
                        }
                    }
                }
            }
            SuspendKind::LetElseAwait {
                pat_binds,
                before_await,
                ..
            } => {
                // Else path does not bind `pat`; success path uses join with pat_binds.
                let _ = pat_binds;
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::For {
                item,
                into_ty,
                before_await,
                ..
            } => {
                let iter_ty = into_iter_ty(into_ty);
                let item_ty = into_item_ty(into_ty);
                upsert_binding(
                    &mut live,
                    Binding {
                        name: format_ident!("__iter"),
                        ty: iter_ty,
                        mutable: true,
                    },
                );
                upsert_binding(
                    &mut live,
                    Binding {
                        name: item.clone(),
                        ty: item_ty,
                        mutable: false,
                    },
                );
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::MatchArm {
                scrut_ty,
                pat_binds,
                before_await,
                ..
            } => {
                for name in pat_binds {
                    upsert_binding(
                        &mut live,
                        Binding {
                            name: name.clone(),
                            ty: scrut_ty.clone(),
                            mutable: false,
                        },
                    );
                }
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::MatchGuard { scrut_ty, .. } => {
                upsert_binding(
                    &mut live,
                    Binding {
                        name: format_ident!("__scrut"),
                        ty: scrut_ty.clone(),
                        mutable: false,
                    },
                );
            }
            SuspendKind::LabeledBlock { before_await, .. }
            | SuspendKind::TryBlock { before_await, .. } => {
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            _ => {}
        }
        captures_at_await.push(live.clone());
        // Locals after resume inside loop/if/match/block bodies do not escape the
        // construct; only `Plain` after-resume stmts stay live for later awaits.
        if after_resume_escapes(&ap.kind) {
            for stmt in after_resume_stmts(ap) {
                if let Some(b) = typed_let_binding(stmt) {
                    upsert_binding(&mut live, b);
                }
            }
        }
        // Pattern bindings from let-else init / if-let scrutinee become live after resume.
        for b in pat_binds_after_resume(ap) {
            upsert_binding(&mut live, b);
        }
        if let Some((name, ok_ty)) = &ap.try_ok {
            upsert_binding(
                &mut live,
                Binding {
                    name: name.clone(),
                    ty: ok_ty.clone(),
                    mutable: false,
                },
            );
        }
        // Loop/block expression value is live for join_stmts and later awaits.
        if let Some(b) = join_value_binding(ap) {
            upsert_binding(&mut live, b);
        }
        // Bindings introduced in join_stmts become live before the next await.
        if let Some(join) = join_stmts_of(ap) {
            for stmt in join {
                if let Some(b) = typed_let_binding(stmt) {
                    upsert_binding(&mut live, b);
                }
            }
        }
    }

    let mut variants = vec![quote! { NotStarted }];
    for (i, (ap, caps)) in awaits.iter().zip(captures_at_await.iter()).enumerate() {
        if for_has_iter_await(&ap.kind) {
            let iter_var = waiting_iter_variant(i);
            let join_fields = join_caps_at[i].iter().map(|b| field_tokens(b));
            let into_ty = for_into_ty(&ap.kind).expect("for iter await");
            let wait_skip = cfg!(feature = "serde") && is_skip_serde(into_ty);
            let wait_field = if wait_skip {
                quote! {
                    #[serde(skip)]
                    __wait: ::core::option::Option<#into_ty>,
                }
            } else {
                quote! {
                    __wait: ::core::option::Option<#into_ty>,
                }
            };
            variants.push(quote! {
                #iter_var {
                    #(#join_fields,)*
                    #wait_field
                }
            });
        }

        if !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. }) {
            let var = waiting_variant(&ap.name);
            let cap_fields = caps.iter().map(|b| field_tokens(b));
            if let Some(child_ty) = &ap.nested_child {
                variants.push(quote! {
                    #var {
                        #(#cap_fields,)*
                        __child: #child_ty,
                    }
                });
            } else {
                let wait_ty = &ap.wait_ty;
                let wait_skip = cfg!(feature = "serde") && is_skip_serde(&ap.wait_ty);
                let wait_field = if wait_skip {
                    quote! {
                        #[serde(skip)]
                        __wait: ::core::option::Option<#wait_ty>,
                    }
                } else {
                    quote! {
                        __wait: ::core::option::Option<#wait_ty>,
                    }
                };
                variants.push(quote! {
                    #var {
                        #(#cap_fields,)*
                        #wait_field
                    }
                });
            }
        }

        if is_loop_kind(&ap.kind) {
            let head_var = loop_head_variant(i);
            let head_caps = loop_head_caps(ap, &join_caps_at[i]);
            let head_fields = head_caps.iter().map(|b| field_tokens(b));
            variants.push(quote! {
                #head_var {
                    #(#head_fields,)*
                }
            });
        }

        if needs_join(&ap.kind) {
            let after_var = join_variant(&ap.kind, i);
            let join_caps = effective_join_caps(ap, &join_caps_at[i]);
            let join_fields = join_caps.iter().map(|b| field_tokens(b));
            variants.push(quote! {
                #after_var {
                    #(#join_fields,)*
                }
            });
        }
    }
    variants.push(quote! { Finished });

    let mut settle_arms = Vec::new();
    for (i, ap) in awaits.iter().enumerate() {
        if for_has_iter_await(&ap.kind) {
            let iter_var = waiting_iter_variant(i);
            let into_ty = for_into_ty(&ap.kind).unwrap();
            settle_arms.push(quote! {
                Self::#iter_var { __wait, .. } => {
                    let value = value
                        .downcast_ref::<#into_ty>()
                        .unwrap_or_else(|| panic!("settle_wait: expected {}", ::core::any::type_name::<#into_ty>()));
                    *__wait = ::core::option::Option::Some(::core::clone::Clone::clone(value));
                }
            });
        }
        if !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. }) {
            let var = waiting_variant(&ap.name);
            if ap.nested_child.is_some() {
                settle_arms.push(quote! {
                    Self::#var { __child, .. } => {
                        __child.settle_wait(value);
                    }
                });
            } else {
                let ty = &ap.wait_ty;
                settle_arms.push(quote! {
                    Self::#var { __wait, .. } => {
                        let value = value
                            .downcast_ref::<#ty>()
                            .unwrap_or_else(|| panic!("settle_wait: expected {}", ::core::any::type_name::<#ty>()));
                        *__wait = ::core::option::Option::Some(*value);
                    }
                });
            }
        }
    }

    let mut all_skips = collect_skip_bindings(&captures_at_await);
    for caps in &join_caps_at {
        for b in caps {
            if is_skip_serde(&b.ty) {
                upsert_binding(&mut all_skips, b.clone());
            }
        }
    }
    let rehyd_name = format_ident!("{}Rehydration", enum_name);

    let mut step_arms = Vec::new();

    if awaits.is_empty() {
        let body = emit_completion_stmts(&after_last, &ready_ok);
        step_arms.push(quote! {
            Self::NotStarted => {
                #body
            }
        });
    } else {
        let enter = gen_enter_await(
            0,
            &awaits[0],
            &waiting_variant(&awaits[0].name),
            &captures_at_await[0],
            &join_caps_at[0],
            &ready_ok,
        );
        step_arms.push(quote! {
            Self::NotStarted => {
                #enter
            }
        });
    }

    for i in 0..awaits.len() {
        let ap = &awaits[i];
        let var = waiting_variant(&ap.name);
        let caps = &captures_at_await[i];
        let tmp = &ap.tmp;

        if for_has_iter_await(&ap.kind) {
            let iter_var = waiting_iter_variant(i);
            let join_pats: Vec<_> = join_caps_at[i].iter().map(cap_pat).collect();
            let join_moves = join_caps_at[i].iter().map(|b| {
                let n = &b.name;
                quote! { #n }
            });
            let head_var = loop_head_variant(i);
            step_arms.push(quote! {
                Self::#iter_var { #(#join_pats,)* __wait } => {
                    let __iterable = __wait.expect("call settle_wait before step");
                    let mut __iter =
                        ::core::iter::IntoIterator::into_iter(__iterable);
                    *self = Self::#head_var {
                        __iter,
                        #(#join_moves,)*
                    };
                    continue 'step;
                }
            });
        }

        if is_loop_kind(&ap.kind) {
            let head_var = loop_head_variant(i);
            let head_caps = loop_head_caps(ap, &join_caps_at[i]);
            let head_pats: Vec<_> = head_caps.iter().map(cap_pat).collect();
            match &ap.kind {
                SuspendKind::For {
                    item,
                    before_await,
                    has_body_await,
                    label,
                    ..
                } => {
                    let goto_after = gen_goto_join(ap, i, &join_caps_at[i]);
                    let before_await_toks = rewrite_loop_body_stmts(
                        i,
                        before_await,
                        &join_caps_at[i],
                        label.as_ref(),
                        true,
                        None,
                        &ready_ok,
                    );
                    let some_body = if *has_body_await {
                        let go_wait = gen_enter_wait(ap, &var, caps);
                        quote! {
                            #before_await_toks
                            #go_wait
                        }
                    } else {
                        let goto_head = gen_goto_loop_head(i, ap, &join_caps_at[i]);
                        quote! {
                            #before_await_toks
                            #goto_head
                        }
                    };
                    step_arms.push(quote! {
                        Self::#head_var { #(#head_pats,)* } => {
                            match ::core::iter::Iterator::next(&mut __iter) {
                                ::core::option::Option::Some(#item) => {
                                    #some_body
                                }
                                ::core::option::Option::None => {
                                    #goto_after
                                }
                            }
                        }
                    });
                }
                SuspendKind::Loop {
                    before_await,
                    label,
                    break_bind,
                    ..
                } => {
                    let go_wait = gen_enter_wait(ap, &var, caps);
                    let before_await_toks = rewrite_loop_body_stmts(
                        i,
                        before_await,
                        &join_caps_at[i],
                        label.as_ref(),
                        false,
                        break_bind.as_ref(),
                        &ready_ok,
                    );
                    step_arms.push(quote! {
                        Self::#head_var { #(#head_pats,)* } => {
                            #before_await_toks
                            #go_wait
                        }
                    });
                }
                SuspendKind::While {
                    sync_cond: Some(cond),
                    before_await,
                    label,
                    has_body_await: true,
                    ..
                } => {
                    let go_wait = gen_enter_wait(ap, &var, caps);
                    let goto_after = gen_goto_join(ap, i, &join_caps_at[i]);
                    let before_await_toks = rewrite_loop_body_stmts(
                        i,
                        before_await,
                        &join_caps_at[i],
                        label.as_ref(),
                        false,
                        None,
                        &ready_ok,
                    );
                    step_arms.push(quote! {
                        Self::#head_var { #(#head_pats,)* } => {
                            if #cond {
                                #before_await_toks
                                #go_wait
                            } else {
                                #goto_after
                            }
                        }
                    });
                }
                SuspendKind::While {
                    sync_cond: None,
                    label,
                    ..
                } => {
                    let go_wait = gen_enter_wait(ap, &var, caps);
                    let _ = label;
                    step_arms.push(quote! {
                        Self::#head_var { #(#head_pats,)* } => {
                            #go_wait
                        }
                    });
                }
                _ => {}
            }
        }

        if !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. }) {
            let cap_pats: Vec<_> = caps.iter().map(cap_pat).collect();
            let cap_moves: Vec<_> = caps
                .iter()
                .map(|b| {
                    let n = &b.name;
                    quote! { #n }
                })
                .collect();
            let guard = rehydration_guard(&rehyd_name, &var, caps);
            let after_resume = gen_after_resume(i, ap, &join_caps_at[i], &ready_ok);
            let tail = match &ap.kind {
                SuspendKind::While { sync_cond: None, .. } => quote! {},
                SuspendKind::Loop { .. }
                | SuspendKind::While { .. }
                | SuspendKind::For { .. } => {
                    gen_goto_loop_head(i, ap, &join_caps_at[i])
                }
                SuspendKind::IfThen { .. }
                | SuspendKind::IfElse { .. }
                | SuspendKind::MatchArm { .. }
                | SuspendKind::MatchGuard { .. }
                | SuspendKind::LabeledBlock { .. }
                | SuspendKind::TryBlock { .. } => {
                    gen_goto_join(ap, i, &effective_join_caps(ap, &join_caps_at[i]))
                }
                // Else-path of let…else diverges after resume (see gen_after_resume).
                SuspendKind::LetElseAwait { .. } => quote! {},
                _ => gen_join_tail(
                    i,
                    &awaits,
                    &captures_at_await,
                    &join_caps_at,
                    &after_last,
                    &ready_ok,
                ),
            };

            if ap.nested_child.is_some() {
                let cap_moves_pending = cap_moves.clone();
                let cap_moves_err = cap_moves;
                step_arms.push(quote! {
                    Self::#var { #(#cap_pats,)* mut __child } => {
                        #guard
                        match __child.step() {
                            ::core::result::Result::Ok(::core::task::Poll::Pending) => {
                                *self = Self::#var {
                                    #(#cap_moves_pending,)*
                                    __child,
                                };
                                break 'step ::core::result::Result::Ok(
                                    ::core::task::Poll::Pending,
                                );
                            }
                            ::core::result::Result::Ok(::core::task::Poll::Ready(#tmp)) => {
                                #after_resume
                                #tail
                            }
                            ::core::result::Result::Err(_) => {
                                *self = Self::#var {
                                    #(#cap_moves_err,)*
                                    __child,
                                };
                                panic!(
                                    "nested #[corot] rehydration is not supported yet"
                                );
                            }
                        }
                    }
                });
            } else {
                step_arms.push(quote! {
                    Self::#var { #(#cap_pats,)* __wait } => {
                        #guard
                        let #tmp = __wait.expect("call settle_wait before step");
                        #after_resume
                        #tail
                    }
                });
            }
        }

        if needs_join(&ap.kind) {
            let after_var = join_variant(&ap.kind, i);
            let join_caps = effective_join_caps(ap, &join_caps_at[i]);
            let join_pats: Vec<_> = join_caps.iter().map(cap_pat).collect();
            let join_stmts = join_stmts_of(ap).unwrap_or(&[]);
            // Final join owns the remaining body (including trailing `Ok(())`).
            let join_body = if i + 1 >= awaits.len() {
                emit_completion_stmts(join_stmts, &ready_ok)
            } else {
                let join_toks = emit_stmts_rewrite_returns(join_stmts, &ready_ok);
                let after_join = gen_join_tail(
                    i,
                    &awaits,
                    &captures_at_await,
                    &join_caps_at,
                    &after_last,
                    &ready_ok,
                );
                quote! {
                    #join_toks
                    #after_join
                }
            };
            step_arms.push(quote! {
                Self::#after_var { #(#join_pats,)* } => {
                    #join_body
                }
            });
        }
    }

    step_arms.push(quote! {
        Self::Finished => {
            break 'step ::core::result::Result::Ok(#ready_ok);
        }
    });

    let settle_fn = quote! {
        pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
            match self {
                #(#settle_arms)*
                _ => panic!("settle_wait called when not waiting"),
            }
        }
    };

    let serde_attrs = if cfg!(feature = "serde") {
        quote! {
            #[derive(::serde::Serialize, ::serde::Deserialize)]
        }
    } else {
        quote! {}
    };

    let (rehyd_enum, rehyd_method, step_ret) = make_rehydration(
        &vis,
        &rehyd_name,
        &awaits,
        &captures_at_await,
        &join_caps_at,
        &all_skips,
        &output_ty,
    );
    let getters = make_getters(&awaits, &captures_at_await, &join_caps_at);

    Ok(quote! {
        #serde_attrs
        #[allow(dead_code)]
        #vis enum #enum_name {
            #(#variants,)*
        }

        #rehyd_enum

        impl #enum_name {
            #settle_fn

            #rehyd_method

            #getters

            #[allow(unused_variables)]
            pub fn step(&mut self) -> #step_ret {
                'step: loop {
                    match ::core::mem::replace(self, Self::Finished) {
                        #(#step_arms,)*
                    }
                }
            }
        }

        #vis fn #fn_name() -> #enum_name {
            #enum_name::NotStarted
        }
    })
}

fn to_upper_camel_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn coroutine_name(fn_name: &Ident) -> Ident {
    format_ident!("{}Coroutine", to_upper_camel_case(&fn_name.to_string()))
}

fn waiting_variant(bind: &Ident) -> Ident {
    format_ident!("Waiting{}", to_upper_camel_case(&bind.to_string()))
}

fn needs_rehydration_variant(field: &Ident) -> Ident {
    format_ident!(
        "NeedsRehydration{}",
        to_upper_camel_case(&field.to_string())
    )
}

fn split_awaits(
    stmts: &[Stmt],
    err_ty: Option<&Type>,
) -> syn::Result<(Vec<AwaitPoint>, Vec<Stmt>)> {
    let mut awaits = Vec::new();
    let mut current: Vec<Stmt> = Vec::new();

    for stmt in stmts {
        if let Some(ap) = as_await_stmt(stmt, awaits.len(), err_ty)? {
            let mut ap = ap;
            ap.before = std::mem::take(&mut current);
            awaits.push(ap);
        } else if stmt_contains_await(stmt) {
            return Err(syn::Error::new_spanned(
                stmt,
                "#[corot] unsupported await placement (supported: typed let; if; match; loop; while; for; labeled block; try)",
            ));
        } else {
            current.push(stmt.clone());
        }
    }

    Ok((awaits, current))
}

fn as_await_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    if let Some(ap) = as_await_try_block_stmt(stmt, index, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_labeled_block_stmt(stmt, index, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_loop_stmt(stmt, index, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(plain) = as_plain_await_let(stmt, err_ty, index)? {
        return Ok(Some(AwaitPoint {
            name: plain.name,
            tmp: plain.tmp,
            wait_ty: plain.wait_ty,
            base: plain.base,
            before: Vec::new(),
            try_ok: plain.try_ok,
            nested_child: plain.nested_child,
            kind: SuspendKind::Plain {
                after_resume: plain.after_resume,
            },
        }));
    }
    if let Some(ap) = as_await_let_else_stmt(stmt, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_if_stmt(stmt, index, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_while_stmt(stmt, index, err_ty)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_for_stmt(stmt, err_ty)? {
        return Ok(Some(ap));
    }
    as_await_match_stmt(stmt, index, err_ty)
}

fn as_plain_await_let(
    stmt: &Stmt,
    err_ty: Option<&Type>,
    await_index: usize,
) -> syn::Result<Option<PlainAwait>> {
    let Stmt::Local(Local {
        attrs,
        let_token,
        pat,
        init: Some(LocalInit {
            eq_token,
            expr,
            diverge,
        }),
        semi_token,
    }) = stmt
    else {
        return Ok(None);
    };

    let else_has_await = diverge
        .as_ref()
        .is_some_and(|(_, e)| contains_await(e));
    let init_has_await = contains_await(expr);

    if !init_has_await && !else_has_await {
        return Ok(None);
    }

    // Await only in else → handled by as_await_let_else_stmt.
    if else_has_await && !init_has_await {
        return Ok(None);
    }

    if else_has_await && init_has_await {
        return Err(syn::Error::new_spanned(
            stmt,
            "#[corot] supports at most one await in let…else (initializer or else, not both)",
        ));
    }

    let has_try = outer_try(expr).is_some();
    let work_expr: Expr = if let Some(inner) = outer_try(expr) {
        inner.clone()
    } else {
        expr.as_ref().clone()
    };

    if has_try && diverge.is_some() {
        return Err(syn::Error::new_spanned(
            stmt,
            "#[corot] `await?` with `let…else` is not supported",
        ));
    }

    let (name, wait_ty, try_ok) = if has_try {
        let Some(err) = err_ty else {
            return Err(syn::Error::new_spanned(
                stmt,
                "#[corot] `await?` requires the async fn to return `Result<(), E>`",
            ));
        };
        let (name, ok_ty) = match pat {
            Pat::Type(PatType { pat, ty, .. }) => (pat_ident_or_discard(pat)?, ty.as_ref().clone()),
            _ => {
                return Err(syn::Error::new_spanned(
                    pat,
                    "`await?` bindings must be `let name: OkType = <expr>.await?`",
                ));
            }
        };
        (
            name.clone(),
            syn::parse_quote!(::core::result::Result<#ok_ty, #err>),
            Some((name, ok_ty)),
        )
    } else {
        // Await in initializer (optional sync else) — allow `let Some(x) = … else`.
        let wait_ty = resolve_let_wait_ty(pat, expr)?;
        let name = match pat {
            Pat::Type(PatType { pat, .. }) => pat_ident_or_discard(pat)?,
            Pat::Ident(p) if p.subpat.is_none() => {
                return Err(syn::Error::new_spanned(
                    pat,
                    "await bindings must be written as `let name: Type = <expr with await>`",
                ));
            }
            _ => format_ident!("letelse"),
        };
        (name, wait_ty, None)
    };

    // `let _: T = …` would collide across awaits; uniquify the stem.
    let name = if name == "_unit" {
        format_ident!("_unit{}", await_index)
    } else {
        name
    };

    let tmp = format_ident!("__await_{}", name);
    let mut resume_expr = work_expr;
    let base = replace_first_await(&mut resume_expr, ident_expr(&tmp))?;
    resume_expr = strip_val_call(resume_expr);

    let nested_child = as_corot_call(&base).map(|(ty, _)| ty);
    if has_try && nested_child.is_some() {
        return Err(syn::Error::new_spanned(
            stmt,
            "#[corot] `await?` on `call::<Child>(…)` is not supported yet",
        ));
    }
    // Keep `call::<Child>(…)` as the base so evaluating it constructs the child.
    let base = if nested_child.is_some() {
        base
    } else {
        strip_val_call(base)
    };

    let after_resume = if try_ok.is_some() {
        // Emitted specially in gen_after_resume (expands `?` to early Ready(Err)).
        Vec::new()
    } else {
        vec![Stmt::Local(Local {
            attrs: attrs.clone(),
            let_token: *let_token,
            pat: pat.clone(),
            init: Some(LocalInit {
                eq_token: *eq_token,
                expr: Box::new(resume_expr),
                diverge: diverge.as_ref().map(|(t, e)| (*t, e.clone())),
            }),
            semi_token: *semi_token,
        })]
    };

    Ok(Some(PlainAwait {
        name,
        tmp,
        wait_ty,
        base,
        after_resume,
        try_ok,
        nested_child,
    }))
}

fn outer_try(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Try(t) => Some(&t.expr),
        Expr::Paren(p) => outer_try(&p.expr),
        Expr::Group(g) => outer_try(&g.expr),
        _ => None,
    }
}

fn as_await_let_else_stmt(
    stmt: &Stmt,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Local(Local {
        pat,
        init: Some(LocalInit {
            expr,
            diverge: Some((_, else_expr)),
            ..
        }),
        ..
    }) = stmt
    else {
        return Ok(None);
    };

    if contains_await(expr) {
        return Ok(None); // init await handled as plain
    }
    if !contains_await(else_expr) {
        return Ok(None);
    }

    let else_stmts = else_block_stmts(else_expr)?;
    let (before_await, plain, after_await) =
        extract_single_await_from_stmts(else_stmts, err_ty)?;
    let scrut_ty = resolve_scrut_ty(pat, expr)?;
    let pat_binds = bindings_from_pat(pat, &scrut_ty)?;

        Ok(Some(AwaitPoint {
        name: plain.name,
        tmp: plain.tmp,
        wait_ty: plain.wait_ty,
        base: plain.base,
        before: Vec::new(),
        try_ok: plain.try_ok,
        nested_child: plain.nested_child,
        kind: SuspendKind::LetElseAwait {
            pat: pat.clone(),
            init: strip_val_call(expr.as_ref().clone()),
            pat_binds,
            before_await,
            after_await: {
                let mut v = plain.after_resume;
                v.extend(after_await);
                v
            },
            join_stmts: Vec::new(),
        },
    }))
}

fn as_await_if_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::If(expr_if), _) = stmt else {
        return Ok(None);
    };

    let cond_has = contains_await(&expr_if.cond);
    let then_has = expr_if.then_branch.stmts.iter().any(stmt_contains_await);
    let else_has = expr_if
        .else_branch
        .as_ref()
        .is_some_and(|(_, e)| contains_await(e));

    match (cond_has, then_has, else_has) {
        (false, false, false) => Ok(None),
        (true, false, false) => {
            // `if let PAT = EXPR.await` or `if EXPR.await`
            if let Expr::Let(expr_let) = expr_if.cond.as_ref() {
                let base = await_base_from_scrut(&expr_let.expr)?;
                let wait_ty = resolve_scrut_ty(&expr_let.pat, &expr_let.expr)?;
                let name = format_ident!("scrut{}", index);
                let tmp = format_ident!("__await_{}", name);
                    Ok(Some(AwaitPoint {
                    name,
                    tmp,
                    wait_ty,
                    base,
                    before: Vec::new(),
                    try_ok: None,
                    nested_child: None,
                    kind: SuspendKind::IfLetScrutinee {
                        pat: expr_let.pat.as_ref().clone(),
                        then_branch: expr_if.then_branch.clone(),
                        else_branch: expr_if.else_branch.as_ref().map(|(_, e)| e.clone()),
                    },
                }))
            } else {
                let Some(base_of_await) = bare_await_base(&expr_if.cond) else {
                    return Err(syn::Error::new_spanned(
                        &expr_if.cond,
                        "await in `if` condition must be a bare `expr.await` (result type: bool)",
                    ));
                };
                let name = format_ident!("cond{}", index);
                let tmp = format_ident!("__await_{}", name);
                    Ok(Some(AwaitPoint {
                    name,
                    tmp: tmp.clone(),
                    wait_ty: syn::parse_quote!(bool),
                    base: base_of_await,
                    before: Vec::new(),
                    try_ok: None,
                    nested_child: None,
                    kind: SuspendKind::IfCondition {
                        resume_cond: ident_expr(&tmp),
                        then_branch: expr_if.then_branch.clone(),
                        else_branch: expr_if.else_branch.as_ref().map(|(_, e)| e.clone()),
                    },
                }))
            }
        }
        (false, true, false) => {
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(&expr_if.then_branch.stmts, err_ty)?;
            let pat_binds = if_let_pat_binds(&expr_if.cond)?;
                Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
                try_ok: plain.try_ok,
                nested_child: plain.nested_child,
                kind: SuspendKind::IfThen {
                    cond: strip_val_in_if_cond(expr_if.cond.as_ref().clone()),
                    pat_binds,
                    before_await,
                    after_await: {
                        let mut v = plain.after_resume;
                        v.extend(after_await);
                        v
                    },
                    else_branch: expr_if.else_branch.as_ref().map(|(_, e)| e.clone()),
                    join_stmts: Vec::new(),
                },
            }))
        }
        (false, false, true) => {
            let else_expr = expr_if
                .else_branch
                .as_ref()
                .map(|(_, e)| e.as_ref())
                .unwrap();
            let parsed = parse_else_await_chain(else_expr, err_ty)?;
                Ok(Some(AwaitPoint {
                name: parsed.name,
                tmp: parsed.tmp,
                wait_ty: parsed.wait_ty,
                base: parsed.base,
                before: Vec::new(),
                try_ok: None,
                nested_child: None,
                kind: SuspendKind::IfElse {
                    cond: strip_val_in_if_cond(expr_if.cond.as_ref().clone()),
                    then_branch: expr_if.then_branch.clone(),
                    else_suspend: parsed.else_suspend,
                    after_await: parsed.after_await,
                    join_stmts: Vec::new(),
                },
            }))
        }
        _ => Err(syn::Error::new_spanned(
            stmt,
            "#[corot] supports at most one await in an if/if-let, and only in the \
             condition/scrutinee, or only in then, or only in else",
        )),
    }
}

fn as_await_loop_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    // `let name: Ty = ['lab:] loop { … }`
    if let Stmt::Local(Local {
        pat,
        init: Some(LocalInit { expr, diverge, .. }),
        ..
    }) = stmt
    {
        if diverge.is_some() {
            return Ok(None);
        }
        let Expr::Loop(expr_loop) = unwrap_parens_ref(expr) else {
            return Ok(None);
        };
        if !expr_loop.body.stmts.iter().any(stmt_contains_await) {
            return Ok(None);
        }
        let Pat::Type(PatType { pat: inner, ty, .. }) = pat else {
            return Err(syn::Error::new_spanned(
                pat,
                "loop-as-expression await bindings must be `let name: Type = loop { … }`",
            ));
        };
        let bind_name = pat_ident_or_discard(inner)?;
        let bind_name = if bind_name == "_unit" {
            format_ident!("_loop{}", index)
        } else {
            bind_name
        };
        let label = expr_loop
            .label
            .as_ref()
            .map(|l| l.name.ident.clone());
        let (before_await, plain, after_await) =
            extract_single_await_from_stmts(&expr_loop.body.stmts, err_ty)?;
        return Ok(Some(AwaitPoint {
            name: plain.name,
            tmp: plain.tmp,
            wait_ty: plain.wait_ty,
            base: plain.base,
            before: Vec::new(),
            try_ok: plain.try_ok,
            nested_child: plain.nested_child,
            kind: SuspendKind::Loop {
                label,
                break_bind: Some((bind_name, ty.as_ref().clone())),
                before_await,
                after_await: {
                    let mut v = plain.after_resume;
                    v.extend(after_await);
                    v
                },
                join_stmts: Vec::new(),
            },
        }));
    }

    // `['lab:] loop { … };` as a statement
    let Stmt::Expr(Expr::Loop(expr_loop), _) = stmt else {
        return Ok(None);
    };
    if !expr_loop.body.stmts.iter().any(stmt_contains_await) {
        return Ok(None);
    }
    let label = expr_loop
        .label
        .as_ref()
        .map(|l| l.name.ident.clone());
    let (before_await, plain, after_await) =
        extract_single_await_from_stmts(&expr_loop.body.stmts, err_ty)?;
    Ok(Some(AwaitPoint {
        name: plain.name,
        tmp: plain.tmp,
        wait_ty: plain.wait_ty,
        base: plain.base,
        before: Vec::new(),
        try_ok: plain.try_ok,
        nested_child: plain.nested_child,
        kind: SuspendKind::Loop {
            label,
            break_bind: None,
            before_await,
            after_await: {
                let mut v = plain.after_resume;
                v.extend(after_await);
                v
            },
            join_stmts: Vec::new(),
        },
    }))
}

fn as_await_while_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::While(expr_while), _) = stmt else {
        return Ok(None);
    };

    let label = expr_while
        .label
        .as_ref()
        .map(|l| l.name.ident.clone());
    let cond_has = contains_await(&expr_while.cond);
    let body_has = expr_while.body.stmts.iter().any(stmt_contains_await);

    match (cond_has, body_has) {
        (false, false) => Ok(None),
        (true, true) => Err(syn::Error::new_spanned(
            stmt,
            "#[corot] supports at most one await in a while/while-let \
             (condition/scrutinee or body, not both)",
        )),
        (false, true) => {
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(&expr_while.body.stmts, err_ty)?;
            let (sync_cond, pat_binds) = match expr_while.cond.as_ref() {
                Expr::Let(expr_let) => {
                    let scrut_ty = resolve_scrut_ty(&expr_let.pat, &expr_let.expr)?;
                    let pat_binds = bindings_from_pat(&expr_let.pat, &scrut_ty)?;
                    let mut expr_let = expr_let.clone();
                    expr_let.expr = Box::new(strip_val_call(*expr_let.expr));
                    (Expr::Let(expr_let), pat_binds)
                }
                other => (other.clone(), Vec::new()),
            };
            Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
                try_ok: plain.try_ok,
                nested_child: plain.nested_child,
                kind: SuspendKind::While {
                    label,
                    sync_cond: Some(sync_cond),
                    await_let_pat: None,
                    pat_binds,
                    has_body_await: true,
                    before_await,
                    after_await: {
                        let mut v = plain.after_resume;
                        v.extend(after_await);
                        v
                    },
                    join_stmts: Vec::new(),
                },
            }))
        }
        (true, false) => {
            if let Expr::Let(expr_let) = expr_while.cond.as_ref() {
                let base = await_base_from_scrut(&expr_let.expr)?;
                let wait_ty = resolve_scrut_ty(&expr_let.pat, &expr_let.expr)?;
                let name = format_ident!("whilescrut{}", index);
                let tmp = format_ident!("__await_{}", name);
                Ok(Some(AwaitPoint {
                    name,
                    tmp,
                    wait_ty,
                    base,
                    before: Vec::new(),
                    try_ok: None,
                    nested_child: None,
                    kind: SuspendKind::While {
                        label,
                        sync_cond: None,
                        await_let_pat: Some(expr_let.pat.as_ref().clone()),
                        pat_binds: Vec::new(),
                        has_body_await: false,
                        before_await: Vec::new(),
                        after_await: expr_while.body.stmts.clone(),
                        join_stmts: Vec::new(),
                    },
                }))
            } else {
                let Some(base) = bare_await_base(&expr_while.cond) else {
                    return Err(syn::Error::new_spanned(
                        &expr_while.cond,
                        "await in `while` condition must be a bare `expr.await` (result type: bool)",
                    ));
                };
                let name = format_ident!("whilecond{}", index);
                let tmp = format_ident!("__await_{}", name);
                Ok(Some(AwaitPoint {
                    name,
                    tmp,
                    wait_ty: syn::parse_quote!(bool),
                    base,
                    before: Vec::new(),
                    try_ok: None,
                    nested_child: None,
                    kind: SuspendKind::While {
                        label,
                        sync_cond: None,
                        await_let_pat: None,
                        pat_binds: Vec::new(),
                        has_body_await: false,
                        before_await: Vec::new(),
                        after_await: expr_while.body.stmts.clone(),
                        join_stmts: Vec::new(),
                    },
                }))
            }
        }
    }
}

fn as_await_labeled_block_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    // `let name: Ty = 'lab: { … }`
    if let Stmt::Local(Local {
        pat,
        init: Some(LocalInit { expr, diverge, .. }),
        ..
    }) = stmt
    {
        if diverge.is_some() {
            return Ok(None);
        }
        let Expr::Block(expr_block) = unwrap_parens_ref(expr) else {
            return Ok(None);
        };
        let Some(label) = expr_block.label.as_ref() else {
            return Ok(None);
        };
        if !expr_block.block.stmts.iter().any(stmt_contains_await) {
            return Ok(None);
        }
        let Pat::Type(PatType { pat: inner, ty, .. }) = pat else {
            return Err(syn::Error::new_spanned(
                pat,
                "labeled-block await bindings must be `let name: Type = 'label: { … }`",
            ));
        };
        let bind_name = pat_ident_or_discard(inner)?;
        let bind_name = if bind_name == "_unit" {
            format_ident!("_blk{}", index)
        } else {
            bind_name
        };
        let bind_ty = ty.as_ref().clone();
        let (before_await, plain, after_await) =
            extract_single_await_from_stmts(&expr_block.block.stmts, err_ty)?;
        return Ok(Some(AwaitPoint {
            name: plain.name,
            tmp: plain.tmp,
            wait_ty: plain.wait_ty,
            base: plain.base,
            before: Vec::new(),
            try_ok: plain.try_ok,
            nested_child: plain.nested_child,
            kind: SuspendKind::LabeledBlock {
                label: label.name.ident.clone(),
                bind_name,
                bind_ty,
                is_stmt: false,
                before_await,
                after_await: {
                    let mut v = plain.after_resume;
                    v.extend(after_await);
                    v
                },
                join_stmts: Vec::new(),
            },
        }));
    }

    // `'lab: { … };` as a statement
    if let Stmt::Expr(Expr::Block(expr_block), _) = stmt {
        let Some(label) = expr_block.label.as_ref() else {
            return Ok(None);
        };
        if !expr_block.block.stmts.iter().any(stmt_contains_await) {
            return Ok(None);
        }
        let (before_await, plain, after_await) =
            extract_single_await_from_stmts(&expr_block.block.stmts, err_ty)?;
        return Ok(Some(AwaitPoint {
            name: plain.name,
            tmp: plain.tmp,
            wait_ty: plain.wait_ty,
            base: plain.base,
            before: Vec::new(),
            try_ok: plain.try_ok,
            nested_child: plain.nested_child,
            kind: SuspendKind::LabeledBlock {
                label: label.name.ident.clone(),
                bind_name: format_ident!("_blk{}", index),
                bind_ty: syn::parse_quote!(()),
                is_stmt: true,
                before_await,
                after_await: {
                    let mut v = plain.after_resume;
                    v.extend(after_await);
                    v
                },
                join_stmts: Vec::new(),
            },
        }));
    }

    Ok(None)
}

fn as_await_try_block_stmt(
    stmt: &Stmt,
    index: usize,
    _fn_err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    // `let name: Result<T, E> = try { … }`
    let Stmt::Local(Local {
        pat,
        init: Some(LocalInit { expr, diverge, .. }),
        ..
    }) = stmt
    else {
        return Ok(None);
    };
    if diverge.is_some() {
        return Ok(None);
    }
    let Expr::TryBlock(try_block) = unwrap_parens_ref(expr) else {
        return Ok(None);
    };
    if !try_block.block.stmts.iter().any(stmt_contains_await) {
        return Ok(None);
    }
    let Pat::Type(PatType { pat: inner, ty, .. }) = pat else {
        return Err(syn::Error::new_spanned(
            pat,
            "try-block await bindings must be `let name: Result<T, E> = try { … }`",
        ));
    };
    let bind_ty = ty.as_ref().clone();
    let (_ok_ty, err_ty) = parse_result_binding_ty(&bind_ty, pat)?;
    let bind_name = pat_ident_or_discard(inner)?;
    let bind_name = if bind_name == "_unit" {
        format_ident!("_try{}", index)
    } else {
        bind_name
    };
    // Use the try-block's `E` for `await?` settle typing (fn may even return `()`).
    let (before_await, plain, after_await) =
        extract_single_await_from_stmts(&try_block.block.stmts, Some(&err_ty))?;
    Ok(Some(AwaitPoint {
        name: plain.name,
        tmp: plain.tmp,
        wait_ty: plain.wait_ty,
        base: plain.base,
        before: Vec::new(),
        try_ok: plain.try_ok,
        nested_child: plain.nested_child,
        kind: SuspendKind::TryBlock {
            bind_name,
            bind_ty,
            before_await,
            after_await: {
                let mut v = plain.after_resume;
                v.extend(after_await);
                v
            },
            join_stmts: Vec::new(),
        },
    }))
}

fn parse_result_binding_ty(ty: &Type, span: &Pat) -> syn::Result<(Type, Type)> {
    match (result_ok_ty(ty), result_err_ty(ty)) {
        (Some(ok), Some(err)) => Ok((ok, err)),
        _ => Err(syn::Error::new_spanned(
            span,
            "try-block bindings must be `let name: Result<T, E> = try { … }`",
        )),
    }
}

fn as_await_for_stmt(
    stmt: &Stmt,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::ForLoop(expr_for), _) = stmt else {
        return Ok(None);
    };

    let has_iter_await = contains_await(&expr_for.expr);
    let has_body_await = expr_for.body.stmts.iter().any(stmt_contains_await);
    if !has_iter_await && !has_body_await {
        return Ok(None);
    }

    let item = pat_ident(&expr_for.pat)?;
    let (into_ty, source) = resolve_for_iterable(&expr_for.expr)?;
    let (iter_expr, iter_await_base) = match source {
        ForIterSource::Sync(expr) => (Some(expr), None),
        ForIterSource::Await(base) => (None, Some(base)),
    };

    let (before_await, plain, after_await) = if has_body_await {
        extract_single_await_from_stmts(&expr_for.body.stmts, err_ty)?
    } else {
        (
            expr_for.body.stmts.clone(),
            PlainAwait {
                name: format_ident!("iter"),
                tmp: format_ident!("__await_iter"),
                wait_ty: into_ty.clone(),
                base: iter_await_base
                    .clone()
                    .unwrap_or_else(|| syn::parse_quote!(())),
                after_resume: Vec::new(),
                try_ok: None,
                nested_child: None,
            },
            Vec::new(),
        )
    };

    Ok(Some(AwaitPoint {
        name: plain.name,
        tmp: plain.tmp,
        wait_ty: plain.wait_ty,
        base: plain.base,
        before: Vec::new(),
        try_ok: plain.try_ok,
        nested_child: plain.nested_child,
        kind: SuspendKind::For {
            label: expr_for.label.as_ref().map(|l| l.name.ident.clone()),
            item,
            into_ty,
            iter_expr,
            iter_await_base,
            has_body_await,
            before_await,
            after_await: {
                let mut v = plain.after_resume;
                v.extend(after_await);
                v
            },
            join_stmts: Vec::new(),
        },
    }))
}

enum ForIterSource {
    Sync(Expr),
    Await(Expr),
}

/// Resolve `for x in EXPR` into the `IntoIterator` type and the sync/await source.
fn resolve_for_iterable(expr: &Expr) -> syn::Result<(Type, ForIterSource)> {
    // `something.await` (whole `in` expression)
    if let Some(base) = bare_await_base(expr) {
        let (into_ty, inner) = into_ty_from_expr(&base)?;
        return Ok((into_ty, ForIterSource::Await(inner)));
    }

    // `iter::<I>(arg.await)` or `iter::<I>(arg)`
    if let Some((into_ty, arg)) = as_corot_iter_call(expr) {
        if let Some(base) = bare_await_base(&arg) {
            return Ok((into_ty, ForIterSource::Await(unwrap_parens(base))));
        }
        if contains_await(&arg) {
            return Err(syn::Error::new_spanned(
                &arg,
                "#[corot] await inside `iter::<I>(…)` must be a bare `expr.await`",
            ));
        }
        return Ok((into_ty, ForIterSource::Sync(arg)));
    }

    // Range literal sugar: `0..3`
    if let Some(into_ty) = try_range_into_ty(expr) {
        return Ok((into_ty, ForIterSource::Sync(expr.clone())));
    }

    Err(syn::Error::new_spanned(
        expr,
        "#[corot] for-await needs a range literal (`0..3`) or \
         `iter::<I>(…)` / `corot_rs::iter::<I>(…)` where `I` is the IntoIterator \
         type (e.g. `iter::<Vec<i32>>(v)`)",
    ))
}

fn into_ty_from_expr(expr: &Expr) -> syn::Result<(Type, Expr)> {
    if let Some((into_ty, arg)) = as_corot_iter_call(expr) {
        return Ok((into_ty, arg));
    }
    if let Some(into_ty) = try_range_into_ty(expr) {
        return Ok((into_ty, unwrap_parens(expr.clone())));
    }
    Err(syn::Error::new_spanned(
        expr,
        "#[corot] awaited for-iterable must be a range or `iter::<I>(…)`",
    ))
}

/// `iter::<I>(arg)` or `path::iter::<I>(arg)` — identity wrapper for type ascription.
fn as_corot_iter_call(expr: &Expr) -> Option<(Type, Expr)> {
    let expr = match expr {
        Expr::Paren(p) => p.expr.as_ref(),
        Expr::Group(g) => g.expr.as_ref(),
        other => other,
    };
    let Expr::Call(call) = expr else {
        return None;
    };
    if call.args.len() != 1 {
        return None;
    }
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let seg = path.path.segments.last()?;
    if seg.ident != "iter" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    let syn::GenericArgument::Type(ty) = args.args.first()? else {
        return None;
    };
    Some((ty.clone(), call.args.first().unwrap().clone()))
}

fn into_item_ty(into_ty: &Type) -> Type {
    syn::parse_quote!(<#into_ty as ::core::iter::IntoIterator>::Item)
}

fn into_iter_ty(into_ty: &Type) -> Type {
    syn::parse_quote!(<#into_ty as ::core::iter::IntoIterator>::IntoIter)
}

fn try_range_into_ty(expr: &Expr) -> Option<Type> {
    let expr = match expr {
        Expr::Paren(p) => p.expr.as_ref(),
        Expr::Group(g) => g.expr.as_ref(),
        other => other,
    };
    let Expr::Range(range) = expr else {
        return None;
    };
    let ty = int_lit_type(range.start.as_deref())
        .or_else(|| int_lit_type(range.end.as_deref()))
        .unwrap_or_else(|| syn::parse_quote!(i32));
    Some(syn::parse_quote!(::std::ops::Range<#ty>))
}

/// `call::<C>(arg)` / `corot_rs::call::<C>(arg)` — nest another `#[corot]` coroutine.
fn as_corot_call(expr: &Expr) -> Option<(Type, Expr)> {
    let expr = match expr {
        Expr::Paren(p) => p.expr.as_ref(),
        Expr::Group(g) => g.expr.as_ref(),
        other => other,
    };
    let Expr::Call(call) = expr else {
        return None;
    };
    if call.args.len() != 1 {
        return None;
    }
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let seg = path.path.segments.last()?;
    if seg.ident != "call" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    let syn::GenericArgument::Type(ty) = args.args.first()? else {
        return None;
    };
    Some((ty.clone(), call.args.first().unwrap().clone()))
}

/// `val::<T>(arg)` / `corot_rs::val::<T>(arg)` — identity wrapper for type ascription.
fn as_val_call(expr: &Expr) -> Option<(Type, Expr)> {
    let expr = match expr {
        Expr::Paren(p) => p.expr.as_ref(),
        Expr::Group(g) => g.expr.as_ref(),
        other => other,
    };
    let Expr::Call(call) = expr else {
        return None;
    };
    if call.args.len() != 1 {
        return None;
    }
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let seg = path.path.segments.last()?;
    if seg.ident != "val" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    let syn::GenericArgument::Type(ty) = args.args.first()? else {
        return None;
    };
    Some((ty.clone(), call.args.first().unwrap().clone()))
}

fn strip_val_call(expr: Expr) -> Expr {
    if let Some((_, inner)) = as_val_call(&expr) {
        inner
    } else {
        expr
    }
}

/// Await receiver for a scrutinee: `expr.await` or `val::<T>(expr.await)`.
fn await_base_from_scrut(expr: &Expr) -> syn::Result<Expr> {
    if let Some(base) = bare_await_base(expr) {
        return Ok(strip_val_call(base));
    }
    if let Some((_, arg)) = as_val_call(expr) {
        if let Some(base) = bare_await_base(&arg) {
            return Ok(base);
        }
    }
    Err(syn::Error::new_spanned(
        expr,
        "await in `if let` / `let…else` scrutinee must be `expr.await` or \
         `val::<T>(expr.await)`",
    ))
}

fn strip_val_in_if_cond(cond: Expr) -> Expr {
    match cond {
        Expr::Let(mut expr_let) => {
            expr_let.expr = Box::new(strip_val_call(*expr_let.expr));
            Expr::Let(expr_let)
        }
        other => strip_val_call(other),
    }
}

fn if_let_pat_binds(cond: &Expr) -> syn::Result<Vec<Binding>> {
    let Expr::Let(expr_let) = cond else {
        return Ok(Vec::new());
    };
    let scrut_ty = resolve_scrut_ty(&expr_let.pat, &expr_let.expr)?;
    bindings_from_pat(&expr_let.pat, &scrut_ty)
}

fn resolve_let_wait_ty(pat: &Pat, expr: &Expr) -> syn::Result<Type> {
    if let Some((ty, _)) = as_val_call(expr) {
        return Ok(ty);
    }
    if let Pat::Type(pt) = pat {
        return Ok(pt.ty.as_ref().clone());
    }
    if let Some(base) = bare_await_base(expr) {
        if let Some((ty, _)) = as_val_call(&base) {
            return Ok(ty);
        }
        return resolve_scrut_ty(pat, &base);
    }
    resolve_scrut_ty(pat, expr)
}

fn resolve_scrut_ty(pat: &Pat, scrut: &Expr) -> syn::Result<Type> {
    if let Some((ty, _)) = as_val_call(scrut) {
        return Ok(ty);
    }
    if let Some(base) = bare_await_base(scrut) {
        if let Some((ty, _)) = as_val_call(&base) {
            return Ok(ty);
        }
    }
    if let Some(ty) = pattern_type_hint(pat)? {
        return Ok(ty);
    }
    if simple_pat_idents(pat).is_empty() {
        // No bindings to type; a placeholder is fine when unused.
        return Ok(syn::parse_quote!(()));
    }
    Err(syn::Error::new_spanned(
        scrut,
        "#[corot] cannot infer scrutinee type for this pattern; use \
         `corot_rs::val::<T>(…)` or a literal pattern (e.g. `Some(0)`)",
    ))
}

fn bindings_from_pat(pat: &Pat, scrut_ty: &Type) -> syn::Result<Vec<Binding>> {
    match pat {
        Pat::Ident(p) if p.subpat.is_none() => Ok(vec![Binding {
            name: p.ident.clone(),
            ty: scrut_ty.clone(),
            mutable: p.mutability.is_some(),
        }]),
        Pat::Ident(p) => {
            let Some((_, sub)) = &p.subpat else {
                return Ok(Vec::new());
            };
            // `x @ PAT` — x has scrut type; also recurse into PAT.
            let mut out = vec![Binding {
                name: p.ident.clone(),
                ty: scrut_ty.clone(),
                mutable: p.mutability.is_some(),
            }];
            out.extend(bindings_from_pat(sub, scrut_ty)?);
            Ok(out)
        }
        Pat::Type(pt) => {
            let inner = bindings_from_pat(&pt.pat, pt.ty.as_ref())?;
            if inner.is_empty() {
                if let Ok(name) = pat_ident(&pt.pat) {
                    return Ok(vec![Binding {
                        name,
                        ty: pt.ty.as_ref().clone(),
                        mutable: false,
                    }]);
                }
            }
            Ok(inner)
        }
        Pat::TupleStruct(p) if p.path.is_ident("Some") && p.elems.len() == 1 => {
            let inner_ty = option_inner_ty(scrut_ty).ok_or_else(|| {
                syn::Error::new_spanned(
                    pat,
                    "#[corot] `Some(…)` pattern requires scrutinee type `Option<T>` \
                     (use `val::<Option<T>>(…)`)",
                )
            })?;
            bindings_from_pat(&p.elems[0], &inner_ty)
        }
        Pat::TupleStruct(p) if p.path.is_ident("Ok") && p.elems.len() == 1 => {
            let inner_ty = result_ok_ty(scrut_ty).ok_or_else(|| {
                syn::Error::new_spanned(
                    pat,
                    "#[corot] `Ok(…)` pattern requires scrutinee type `Result<T, E>`",
                )
            })?;
            bindings_from_pat(&p.elems[0], &inner_ty)
        }
        Pat::TupleStruct(p) if p.path.is_ident("Err") && p.elems.len() == 1 => {
            let inner_ty = result_err_ty(scrut_ty).ok_or_else(|| {
                syn::Error::new_spanned(
                    pat,
                    "#[corot] `Err(…)` pattern requires scrutinee type `Result<T, E>`",
                )
            })?;
            bindings_from_pat(&p.elems[0], &inner_ty)
        }
        Pat::Or(p) => {
            // Bindings must be the same across arms; take from first case.
            if let Some(first) = p.cases.first() {
                bindings_from_pat(first, scrut_ty)
            } else {
                Ok(Vec::new())
            }
        }
        Pat::Wild(_) | Pat::Lit(_) | Pat::Path(_) => Ok(Vec::new()),
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] unsupported pattern in if-let / let-else await",
        )),
    }
}

fn option_inner_ty(ty: &Type) -> Option<Type> {
    let Type::Path(p) = ty else {
        return None;
    };
    let seg = p.path.segments.last()?;
    if seg.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    }
}

fn result_ok_ty(ty: &Type) -> Option<Type> {
    result_arg(ty, 0)
}

fn result_err_ty(ty: &Type) -> Option<Type> {
    result_arg(ty, 1)
}

fn result_arg(ty: &Type, idx: usize) -> Option<Type> {
    let Type::Path(p) = ty else {
        return None;
    };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    match args.args.iter().nth(idx)? {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    }
}

fn emit_stmts_rewrite_returns(
    stmts: &[Stmt],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let parts = stmts
        .iter()
        .map(|s| emit_stmt_rewrite_returns(s, ready_ok));
    quote! { #(#parts)* }
}

fn emit_stmt_rewrite_returns(
    stmt: &Stmt,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Local(local) => {
            let attrs = &local.attrs;
            let pat = &local.pat;
            if let Some(init) = &local.init {
                let expr = emit_expr_rewrite_returns(&init.expr, ready_ok);
                if let Some((_, diverge)) = &init.diverge {
                    let else_body = emit_expr_rewrite_returns(diverge, ready_ok);
                    return quote! {
                        #(#attrs)*
                        let #pat = #expr else #else_body;
                    };
                }
                return quote! {
                    #(#attrs)*
                    let #pat = #expr;
                };
            }
            quote! { #stmt }
        }
        Stmt::Expr(expr, semi) => {
            // Trailing `Ok(())` / `Err(e)` / `return` must finish the coroutine —
            // do not emit them as values before a following `*self = …`.
            if semi.is_none() {
                if let Some(finish) = as_result_finish_expr(expr, ready_ok) {
                    return finish;
                }
                if let Expr::Return(ret) = expr {
                    return emit_return_finish(ret.expr.as_deref(), ready_ok);
                }
            }
            let e = emit_expr_rewrite_returns(expr, ready_ok);
            match semi {
                Some(_) => quote! { #e; },
                None => quote! { #e },
            }
        }
        other => quote! { #other },
    }
}

fn emit_expr_rewrite_returns(
    expr: &Expr,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match expr {
        Expr::Return(ret) => emit_return_finish(ret.expr.as_deref(), ready_ok),
        Expr::Try(t) => {
            let inner = emit_expr_rewrite_returns(&t.expr, ready_ok);
            emit_question_fn_exit(inner)
        }
        Expr::TryBlock(tb) => emit_sync_try_block(&tb.block.stmts, ready_ok),
        Expr::Block(b) => {
            let label = &b.label;
            let stmts = emit_stmts_rewrite_returns(&b.block.stmts, ready_ok);
            quote! { #label { #stmts } }
        }
        Expr::If(expr_if) => {
            let cond = emit_expr_rewrite_returns(&expr_if.cond, ready_ok);
            let then_stmts = emit_stmts_rewrite_returns(&expr_if.then_branch.stmts, ready_ok);
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #then_stmts
                    }
                },
                Some((_, else_expr)) => {
                    let else_body = emit_expr_rewrite_returns(else_expr, ready_ok);
                    quote! {
                        if #cond {
                            #then_stmts
                        } else #else_body
                    }
                }
            }
        }
        Expr::Match(m) => {
            let scrut = emit_expr_rewrite_returns(&m.expr, ready_ok);
            let arms = m.arms.iter().map(|a| {
                let attrs = &a.attrs;
                let pat = &a.pat;
                let guard = match &a.guard {
                    Some((_, g)) => {
                        let g = emit_expr_rewrite_returns(g, ready_ok);
                        quote! { if #g }
                    }
                    None => quote! {},
                };
                let body = emit_expr_rewrite_returns(&a.body, ready_ok);
                quote! {
                    #(#attrs)*
                    #pat #guard => #body,
                }
            });
            quote! {
                match #scrut {
                    #(#arms)*
                }
            }
        }
        Expr::Loop(l) => {
            let body = emit_stmts_rewrite_returns(&l.body.stmts, ready_ok);
            let label = &l.label;
            quote! {
                #label loop {
                    #body
                }
            }
        }
        Expr::While(w) => {
            let cond = emit_expr_rewrite_returns(&w.cond, ready_ok);
            let body = emit_stmts_rewrite_returns(&w.body.stmts, ready_ok);
            let label = &w.label;
            quote! {
                #label while #cond {
                    #body
                }
            }
        }
        Expr::ForLoop(f) => {
            let pat = &f.pat;
            let iter = emit_expr_rewrite_returns(&f.expr, ready_ok);
            let body = emit_stmts_rewrite_returns(&f.body.stmts, ready_ok);
            let label = &f.label;
            quote! {
                #label for #pat in #iter {
                    #body
                }
            }
        }
        Expr::Call(c) => {
            let func = emit_expr_rewrite_returns(&c.func, ready_ok);
            let args = c
                .args
                .iter()
                .map(|a| emit_expr_rewrite_returns(a, ready_ok));
            quote! { #func(#(#args),*) }
        }
        Expr::MethodCall(m) => {
            let receiver = emit_expr_rewrite_returns(&m.receiver, ready_ok);
            let method = &m.method;
            let turbofish = &m.turbofish;
            let args = m
                .args
                .iter()
                .map(|a| emit_expr_rewrite_returns(a, ready_ok));
            quote! { #receiver.#method #turbofish (#(#args),*) }
        }
        Expr::Binary(b) => {
            let left = emit_expr_rewrite_returns(&b.left, ready_ok);
            let op = &b.op;
            let right = emit_expr_rewrite_returns(&b.right, ready_ok);
            quote! { #left #op #right }
        }
        Expr::Unary(u) => {
            let op = &u.op;
            let expr = emit_expr_rewrite_returns(&u.expr, ready_ok);
            quote! { #op #expr }
        }
        Expr::Paren(p) => {
            let inner = emit_expr_rewrite_returns(&p.expr, ready_ok);
            quote! { (#inner) }
        }
        Expr::Group(g) => emit_expr_rewrite_returns(&g.expr, ready_ok),
        Expr::Reference(r) => {
            let mutability = &r.mutability;
            let expr = emit_expr_rewrite_returns(&r.expr, ready_ok);
            quote! { &#mutability #expr }
        }
        Expr::Field(f) => {
            let base = emit_expr_rewrite_returns(&f.base, ready_ok);
            let member = &f.member;
            quote! { #base.#member }
        }
        Expr::Index(i) => {
            let expr = emit_expr_rewrite_returns(&i.expr, ready_ok);
            let index = emit_expr_rewrite_returns(&i.index, ready_ok);
            quote! { #expr[#index] }
        }
        Expr::Tuple(t) => {
            let elems = t
                .elems
                .iter()
                .map(|e| emit_expr_rewrite_returns(e, ready_ok));
            quote! { (#(#elems),*) }
        }
        Expr::Array(a) => {
            let elems = a
                .elems
                .iter()
                .map(|e| emit_expr_rewrite_returns(e, ready_ok));
            quote! { [#(#elems),*] }
        }
        Expr::Cast(c) => {
            let expr = emit_expr_rewrite_returns(&c.expr, ready_ok);
            let ty = &c.ty;
            quote! { #expr as #ty }
        }
        Expr::Assign(a) => {
            let left = emit_expr_rewrite_returns(&a.left, ready_ok);
            let right = emit_expr_rewrite_returns(&a.right, ready_ok);
            quote! { #left = #right }
        }
        // Closures / async blocks keep native `?` (returns from the closure).
        Expr::Closure(_) | Expr::Async(_) => quote! { #expr },
        other => {
            if let Some(finish) = as_result_finish_expr(other, ready_ok) {
                finish
            } else {
                quote! { #other }
            }
        }
    }
}

/// `expr?` at function level → finish coroutine with `Poll::Ready(Err(…))`.
fn emit_question_fn_exit(inner: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote! {
        match #inner {
            ::core::result::Result::Ok(__v) => __v,
            ::core::result::Result::Err(__e) => {
                *self = Self::Finished;
                break 'step ::core::result::Result::Ok(
                    ::core::task::Poll::Ready(::core::result::Result::Err(
                        ::core::convert::From::from(__e),
                    )),
                );
            }
        }
    }
}

/// Desugar a sync (no-await) `try { … }` to a labeled block with `break` values.
fn emit_sync_try_block(
    stmts: &[Stmt],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    static TRY_LABEL: AtomicUsize = AtomicUsize::new(0);
    let n = TRY_LABEL.fetch_add(1, Ordering::Relaxed);
    let label = Lifetime::new(
        &format!("'__corot_try{n}"),
        proc_macro2::Span::call_site(),
    );
    let (body_stmts, trailing) = split_block_trailing(stmts);
    let body = body_stmts
        .iter()
        .map(|s| rewrite_sync_try_stmt(s, &label, ready_ok));
    let value = match trailing {
        Some(e) => rewrite_sync_try_expr(e, &label, ready_ok),
        None => quote! { () },
    };
    quote! {
        #label: {
            #(#body)*
            break #label (::core::result::Result::Ok(#value));
        }
    }
}

fn rewrite_sync_try_stmt(
    stmt: &Stmt,
    label: &Lifetime,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Local(local) => {
            let attrs = &local.attrs;
            let pat = &local.pat;
            if let Some(init) = &local.init {
                let expr = rewrite_sync_try_expr(&init.expr, label, ready_ok);
                if let Some((_, diverge)) = &init.diverge {
                    let else_body = rewrite_sync_try_expr(diverge, label, ready_ok);
                    return quote! {
                        #(#attrs)*
                        let #pat = #expr else #else_body;
                    };
                }
                return quote! {
                    #(#attrs)*
                    let #pat = #expr;
                };
            }
            quote! { #stmt }
        }
        Stmt::Expr(expr, semi) => {
            let e = rewrite_sync_try_expr(expr, label, ready_ok);
            match semi {
                Some(_) => quote! { #e; },
                None => quote! { #e },
            }
        }
        other => emit_stmt_rewrite_returns(other, ready_ok),
    }
}

fn rewrite_sync_try_expr(
    expr: &Expr,
    label: &Lifetime,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match expr {
        Expr::Try(t) => {
            let inner = rewrite_sync_try_expr(&t.expr, label, ready_ok);
            quote! {
                match #inner {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => {
                        break #label (::core::result::Result::Err(
                            ::core::convert::From::from(__e),
                        ));
                    }
                }
            }
        }
        Expr::Return(ret) => emit_return_finish(ret.expr.as_deref(), ready_ok),
        Expr::TryBlock(tb) => emit_sync_try_block(&tb.block.stmts, ready_ok),
        Expr::Block(b) => {
            let parts = b
                .block
                .stmts
                .iter()
                .map(|s| rewrite_sync_try_stmt(s, label, ready_ok));
            let blk_label = &b.label;
            quote! { #blk_label { #(#parts)* } }
        }
        Expr::If(expr_if) => {
            let cond = rewrite_sync_try_expr(&expr_if.cond, label, ready_ok);
            let then_parts = expr_if
                .then_branch
                .stmts
                .iter()
                .map(|s| rewrite_sync_try_stmt(s, label, ready_ok));
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #(#then_parts)*
                    }
                },
                Some((_, else_expr)) => {
                    let else_body = rewrite_sync_try_expr(else_expr, label, ready_ok);
                    quote! {
                        if #cond {
                            #(#then_parts)*
                        } else #else_body
                    }
                }
            }
        }
        Expr::Match(m) => {
            let scrut = rewrite_sync_try_expr(&m.expr, label, ready_ok);
            let arms = m.arms.iter().map(|arm| {
                let attrs = &arm.attrs;
                let pat = &arm.pat;
                let guard = match &arm.guard {
                    Some((_, g)) => {
                        let g = rewrite_sync_try_expr(g, label, ready_ok);
                        quote! { if #g }
                    }
                    None => quote! {},
                };
                let body = rewrite_sync_try_expr(&arm.body, label, ready_ok);
                quote! {
                    #(#attrs)*
                    #pat #guard => #body,
                }
            });
            quote! {
                match #scrut {
                    #(#arms)*
                }
            }
        }
        Expr::Call(c) => {
            let func = rewrite_sync_try_expr(&c.func, label, ready_ok);
            let args = c
                .args
                .iter()
                .map(|a| rewrite_sync_try_expr(a, label, ready_ok));
            quote! { #func(#(#args),*) }
        }
        Expr::MethodCall(m) => {
            let receiver = rewrite_sync_try_expr(&m.receiver, label, ready_ok);
            let method = &m.method;
            let turbofish = &m.turbofish;
            let args = m
                .args
                .iter()
                .map(|a| rewrite_sync_try_expr(a, label, ready_ok));
            quote! { #receiver.#method #turbofish (#(#args),*) }
        }
        Expr::Binary(b) => {
            let left = rewrite_sync_try_expr(&b.left, label, ready_ok);
            let op = &b.op;
            let right = rewrite_sync_try_expr(&b.right, label, ready_ok);
            quote! { #left #op #right }
        }
        Expr::Paren(p) => {
            let inner = rewrite_sync_try_expr(&p.expr, label, ready_ok);
            quote! { (#inner) }
        }
        Expr::Group(g) => rewrite_sync_try_expr(&g.expr, label, ready_ok),
        Expr::Closure(_) | Expr::Async(_) => quote! { #expr },
        other => emit_expr_rewrite_returns(other, ready_ok),
    }
}

/// Rewrite `return` / `return <expr>` into coroutine completion inside `step`.
fn emit_return_finish(
    value: Option<&Expr>,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match value {
        None => quote! {{
            *self = Self::Finished;
            break 'step ::core::result::Result::Ok(#ready_ok);
        }},
        Some(e) => quote! {{
            *self = Self::Finished;
            break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(#e));
        }},
    }
}

/// Emit stmts then finish with `Ready(Ok(()))` / `Ready(())`, rewriting a trailing
/// `Ok(())` / `Err(e)` / `return …` so they don't sit before `*self = …` (which
/// would parse as multiplication).
fn emit_completion_stmts(
    stmts: &[Stmt],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let default_finish = quote! {
        *self = Self::Finished;
        break 'step ::core::result::Result::Ok(#ready_ok);
    };
    if stmts.is_empty() {
        return default_finish;
    }
    let last = stmts.last().unwrap();
    let prefix = &stmts[..stmts.len() - 1];
    let prefix_toks = emit_stmts_rewrite_returns(prefix, ready_ok);
    if let Some(finish) = as_result_finish_stmt(last, ready_ok) {
        quote! {
            #prefix_toks
            #finish
        }
    } else {
        let last_tok = emit_stmt_rewrite_returns(last, ready_ok);
        quote! {
            #prefix_toks
            #last_tok
            #default_finish
        }
    }
}

fn as_result_finish_stmt(
    stmt: &Stmt,
    ready_ok: &proc_macro2::TokenStream,
) -> Option<proc_macro2::TokenStream> {
    match stmt {
        Stmt::Expr(Expr::Return(ret), _) => {
            Some(emit_return_finish(ret.expr.as_deref(), ready_ok))
        }
        // Trailing expression (no `;`) — typical `Ok(())` / `Err(e)` fn tail.
        Stmt::Expr(expr, None) => as_result_finish_expr(expr, ready_ok),
        _ => None,
    }
}

fn as_result_finish_expr(
    expr: &Expr,
    ready_ok: &proc_macro2::TokenStream,
) -> Option<proc_macro2::TokenStream> {
    if is_ok_unit_expr(expr) {
        return Some(quote! {
            *self = Self::Finished;
            break 'step ::core::result::Result::Ok(#ready_ok);
        });
    }
    if let Some(err) = as_err_call_arg(expr) {
        return Some(quote! {
            *self = Self::Finished;
            break 'step ::core::result::Result::Ok(
                ::core::task::Poll::Ready(::core::result::Result::Err(
                    ::core::convert::From::from(#err),
                )),
            );
        });
    }
    None
}

fn is_ok_unit_expr(expr: &Expr) -> bool {
    let Expr::Call(call) = unwrap_parens_ref(expr) else {
        return false;
    };
    if !path_is_ident(&call.func, "Ok") || call.args.len() != 1 {
        return false;
    }
    matches!(call.args.first(), Some(Expr::Tuple(t)) if t.elems.is_empty())
}

fn as_err_call_arg(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(call) = unwrap_parens_ref(expr) else {
        return None;
    };
    if !path_is_ident(&call.func, "Err") || call.args.len() != 1 {
        return None;
    }
    call.args.first()
}

fn unwrap_parens_ref(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(p) => unwrap_parens_ref(&p.expr),
        Expr::Group(g) => unwrap_parens_ref(&g.expr),
        other => other,
    }
}

fn path_is_ident(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Path(p) => p.path.is_ident(name),
        _ => false,
    }
}

fn stmt_has_return(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(expr, _) => expr_has_return(expr),
        Stmt::Local(local) => local
            .init
            .as_ref()
            .and_then(|i| i.diverge.as_ref())
            .is_some_and(|(_, e)| expr_has_return(e)),
        _ => false,
    }
}

fn expr_has_return(expr: &Expr) -> bool {
    match expr {
        Expr::Return(_) => true,
        Expr::Block(b) => b.block.stmts.iter().any(stmt_has_return),
        Expr::If(e) => {
            e.then_branch.stmts.iter().any(stmt_has_return)
                || e.else_branch
                    .as_ref()
                    .is_some_and(|(_, x)| expr_has_return(x))
        }
        _ => false,
    }
}

fn as_await_match_stmt(
    stmt: &Stmt,
    index: usize,
    err_ty: Option<&Type>,
) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::Match(expr_match), _) = stmt else {
        return Ok(None);
    };

    let scrut_has = contains_await(&expr_match.expr);
    let mut arm_await_idx: Option<usize> = None;
    let mut guard_await_idx: Option<usize> = None;

    for (i, arm) in expr_match.arms.iter().enumerate() {
        if arm
            .guard
            .as_ref()
            .is_some_and(|(_, g)| contains_await(g))
        {
            if guard_await_idx.is_some() {
                return Err(syn::Error::new_spanned(
                    arm,
                    "#[corot] supports at most one await in a match (including guards)",
                ));
            }
            guard_await_idx = Some(i);
        }
        if contains_await(&arm.body) {
            if arm_await_idx.is_some() {
                return Err(syn::Error::new_spanned(
                    arm,
                    "#[corot] supports at most one await in a match",
                ));
            }
            arm_await_idx = Some(i);
        }
    }

    match (scrut_has, arm_await_idx, guard_await_idx) {
        (false, None, None) => Ok(None),
        (true, None, None) => {
            let Some(base) = bare_await_base(&expr_match.expr) else {
                return Err(syn::Error::new_spanned(
                    &expr_match.expr,
                    "await in match scrutinee must be a bare `expr.await`",
                ));
            };
            let scrut_ty = infer_match_scrut_ty(&expr_match.arms)?;
            let name = format_ident!("scrut{}", index);
            let tmp = format_ident!("__await_{}", name);
                Ok(Some(AwaitPoint {
                name,
                tmp: tmp.clone(),
                wait_ty: scrut_ty,
                base,
                before: Vec::new(),
                try_ok: None,
                nested_child: None,
                kind: SuspendKind::MatchScrutinee {
                    arms: expr_match.arms.clone(),
                },
            }))
        }
        (false, Some(ai), None) => {
            let scrut_ty = infer_match_scrut_ty(&expr_match.arms)?;
            let sus = &expr_match.arms[ai];
            if sus
                .guard
                .as_ref()
                .is_some_and(|(_, g)| contains_await(g))
            {
                return Err(syn::Error::new_spanned(
                    sus,
                    "#[corot] await in both a match guard and arm body is not supported",
                ));
            }
            check_simple_match_pat(&sus.pat)?;
            let pat_binds = simple_pat_idents(&sus.pat);
            let stmts = expr_as_stmts(&sus.body);
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(&stmts, err_ty)?;
            let sus_guard = sus.guard.as_ref().map(|(_, g)| g.clone());
                Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
                try_ok: plain.try_ok,
                nested_child: plain.nested_child,
                kind: SuspendKind::MatchArm {
                    scrutinee: expr_match.expr.as_ref().clone(),
                    scrut_ty,
                    pat_binds,
                    arms_before: expr_match.arms[..ai].to_vec(),
                    sus_pat: sus.pat.clone(),
                    sus_guard,
                    before_await,
                    after_await: {
                        let mut v = plain.after_resume;
                        v.extend(after_await);
                        v
                    },
                    arms_after: expr_match.arms[ai + 1..].to_vec(),
                    join_stmts: Vec::new(),
                },
            }))
        }
        (false, None, Some(gi)) => {
            let scrut_ty = infer_match_scrut_ty(&expr_match.arms)?;
            let sus = &expr_match.arms[gi];
            if contains_await(&sus.body) {
                return Err(syn::Error::new_spanned(
                    sus,
                    "#[corot] await in both a match guard and arm body is not supported",
                ));
            }
            check_simple_match_pat(&sus.pat)?;
            let (_, guard_expr) = sus.guard.as_ref().expect("guard await index");
            let Some(base) = bare_await_base(guard_expr) else {
                return Err(syn::Error::new_spanned(
                    guard_expr,
                    "await in match guard must be a bare `expr.await` (result type: bool)",
                ));
            };
            let name = format_ident!("guard{}", index);
            let tmp = format_ident!("__await_{}", name);
                Ok(Some(AwaitPoint {
                name,
                tmp,
                wait_ty: syn::parse_quote!(bool),
                base,
                before: Vec::new(),
                try_ok: None,
                nested_child: None,
                kind: SuspendKind::MatchGuard {
                    scrutinee: expr_match.expr.as_ref().clone(),
                    scrut_ty,
                    arms_before: expr_match.arms[..gi].to_vec(),
                    sus_pat: sus.pat.clone(),
                    sus_body: sus.body.clone(),
                    arms_after: expr_match.arms[gi + 1..].to_vec(),
                    join_stmts: Vec::new(),
                },
            }))
        }
        _ => Err(syn::Error::new_spanned(
            stmt,
            "#[corot] supports at most one await in a match, and only in the scrutinee, \
             or only in one arm body, or only in one guard",
        )),
    }
}

fn expr_as_stmts(expr: &Expr) -> Vec<Stmt> {
    match expr {
        Expr::Block(b) => b.block.stmts.clone(),
        other => vec![Stmt::Expr(other.clone(), None)],
    }
}

fn check_simple_match_pat(pat: &Pat) -> syn::Result<()> {
    match pat {
        Pat::Ident(p) => {
            if let Some((_, sub)) = &p.subpat {
                check_simple_match_pat(sub)?;
            }
            Ok(())
        }
        Pat::Wild(_) | Pat::Lit(_) => Ok(()),
        Pat::Path(p) if p.qself.is_none() => Ok(()),
        Pat::Or(p) => {
            for case in &p.cases {
                check_simple_match_pat(case)?;
            }
            Ok(())
        }
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] match-await currently only supports simple patterns \
             (ident, `ident @ lit`, literal, path, `_`, or `|` of those)",
        )),
    }
}

fn simple_pat_idents(pat: &Pat) -> Vec<Ident> {
    match pat {
        Pat::Ident(p) => vec![p.ident.clone()],
        Pat::Or(p) => {
            let mut out = Vec::new();
            for case in &p.cases {
                for id in simple_pat_idents(case) {
                    if !out.iter().any(|x| x == &id) {
                        out.push(id);
                    }
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn infer_match_scrut_ty(arms: &[syn::Arm]) -> syn::Result<Type> {
    let mut found: Option<Type> = None;
    for arm in arms {
        if let Some(ty) = pattern_type_hint(&arm.pat)? {
            if let Some(prev) = &found {
                if !types_eq(prev, &ty) {
                    return Err(syn::Error::new_spanned(
                        &arm.pat,
                        "#[corot] conflicting pattern type hints in match arms",
                    ));
                }
            } else {
                found = Some(ty);
            }
        }
    }
    found.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[corot] cannot infer match scrutinee type; use a literal pattern \
             (e.g. `0`, `true`) in at least one arm",
        )
    })
}

fn types_eq(a: &Type, b: &Type) -> bool {
    quote!(#a).to_string() == quote!(#b).to_string()
}

fn pattern_type_hint(pat: &Pat) -> syn::Result<Option<Type>> {
    match pat {
        Pat::Lit(p) => match &p.lit {
            Lit::Int(lit) => Ok(Some(int_lit_suffix_type(lit))),
            Lit::Bool(_) => Ok(Some(syn::parse_quote!(bool))),
            Lit::Char(_) => Ok(Some(syn::parse_quote!(char))),
            other => Err(syn::Error::new_spanned(
                other,
                "#[corot] unsupported literal pattern for match type inference",
            )),
        },
        Pat::TupleStruct(p) if p.path.is_ident("Some") && p.elems.len() == 1 => {
            match pattern_type_hint(&p.elems[0])? {
                Some(inner) => Ok(Some(syn::parse_quote!(::core::option::Option<#inner>))),
                None => Ok(None),
            }
        }
        Pat::Path(p) if p.path.is_ident("None") => Ok(None),
        Pat::Or(p) => {
            let mut found = None;
            for case in &p.cases {
                if let Some(ty) = pattern_type_hint(case)? {
                    found = Some(ty);
                    break;
                }
            }
            Ok(found)
        }
        Pat::Ident(p) => {
            if let Some((_, sub)) = &p.subpat {
                pattern_type_hint(sub)
            } else {
                Ok(None)
            }
        }
        Pat::Wild(_) | Pat::Path(_) => Ok(None),
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] cannot infer type from this match pattern",
        )),
    }
}

fn int_lit_suffix_type(lit: &syn::LitInt) -> Type {
    match lit.suffix() {
        "" | "i32" => syn::parse_quote!(i32),
        "i64" => syn::parse_quote!(i64),
        "u32" => syn::parse_quote!(u32),
        "u64" => syn::parse_quote!(u64),
        "usize" => syn::parse_quote!(usize),
        "isize" => syn::parse_quote!(isize),
        _ => syn::parse_quote!(i32),
    }
}

fn int_lit_type(expr: Option<&Expr>) -> Option<Type> {
    let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(lit),
        ..
    }) = expr?
    else {
        return None;
    };
    match lit.suffix() {
        "" | "i32" => Some(syn::parse_quote!(i32)),
        "i64" => Some(syn::parse_quote!(i64)),
        "u32" => Some(syn::parse_quote!(u32)),
        "u64" => Some(syn::parse_quote!(u64)),
        "usize" => Some(syn::parse_quote!(usize)),
        "isize" => Some(syn::parse_quote!(isize)),
        _ => None,
    }
}

fn bare_await_base(cond: &Expr) -> Option<Expr> {
    match cond {
        Expr::Await(a) => Some(a.base.as_ref().clone()),
        Expr::Paren(p) => bare_await_base(&p.expr),
        Expr::Group(g) => bare_await_base(&g.expr),
        _ => None,
    }
}

fn unwrap_parens(expr: Expr) -> Expr {
    match expr {
        Expr::Paren(p) => unwrap_parens(*p.expr),
        Expr::Group(g) => unwrap_parens(*g.expr),
        other => other,
    }
}

fn else_block_stmts(else_expr: &Expr) -> syn::Result<&[Stmt]> {
    match else_expr {
        Expr::Block(b) => Ok(&b.block.stmts),
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] let…else requires `else { ... }` (a block)",
        )),
    }
}

struct ParsedElseAwait {
    name: Ident,
    tmp: Ident,
    wait_ty: Type,
    base: Expr,
    else_suspend: ElseSuspend,
    after_await: Vec<Stmt>,
}

/// Walk `else` / `else if` chain and locate the single await.
fn parse_else_await_chain(
    else_expr: &Expr,
    err_ty: Option<&Type>,
) -> syn::Result<ParsedElseAwait> {
    match else_expr {
        Expr::Block(b) => {
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(&b.block.stmts, err_ty)?;
            Ok(ParsedElseAwait {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                else_suspend: ElseSuspend::FinalBlock { before_await },
                after_await: {
                    let mut v = plain.after_resume;
                    v.extend(after_await);
                    v
                },
            })
        }
        Expr::If(inner) => {
            let cond_has = contains_await(&inner.cond);
            let then_has = inner.then_branch.stmts.iter().any(stmt_contains_await);
            let else_has = inner
                .else_branch
                .as_ref()
                .is_some_and(|(_, e)| contains_await(e));

            match (cond_has, then_has, else_has) {
                (false, true, false) => {
                    let (before_await, plain, after_await) =
                        extract_single_await_from_stmts(&inner.then_branch.stmts, err_ty)?;
                    Ok(ParsedElseAwait {
                        name: plain.name,
                        tmp: plain.tmp,
                        wait_ty: plain.wait_ty,
                        base: plain.base,
                        else_suspend: ElseSuspend::ElseIfThen {
                            cond: strip_val_in_if_cond(inner.cond.as_ref().clone()),
                            before_await,
                            rest_else: inner.else_branch.as_ref().map(|(_, e)| e.clone()),
                        },
                        after_await: {
                            let mut v = plain.after_resume;
                            v.extend(after_await);
                            v
                        },
                    })
                }
                (false, false, true) => {
                    let rest_expr = inner
                        .else_branch
                        .as_ref()
                        .map(|(_, e)| e.as_ref())
                        .unwrap();
                    let mut parsed = parse_else_await_chain(rest_expr, err_ty)?;
                    parsed.else_suspend = ElseSuspend::ElseIfSkip {
                        cond: strip_val_in_if_cond(inner.cond.as_ref().clone()),
                        then_branch: inner.then_branch.clone(),
                        rest: Box::new(parsed.else_suspend),
                    };
                    Ok(parsed)
                }
                (true, false, false) => {
                    if let Expr::Let(_) = inner.cond.as_ref() {
                        return Err(syn::Error::new_spanned(
                            &inner.cond,
                            "#[corot] await in `else if let` scrutinee is not supported yet",
                        ));
                    }
                    let Some(base) = bare_await_base(&inner.cond) else {
                        return Err(syn::Error::new_spanned(
                            &inner.cond,
                            "await in `else if` condition must be a bare `expr.await` (bool)",
                        ));
                    };
                    let name = format_ident!("elseif_cond");
                    let tmp = format_ident!("__await_{}", name);
                    Ok(ParsedElseAwait {
                        name,
                        tmp,
                        wait_ty: syn::parse_quote!(bool),
                        base,
                        else_suspend: ElseSuspend::ElseIfCond {
                            then_branch: inner.then_branch.clone(),
                            rest_else: inner.else_branch.as_ref().map(|(_, e)| e.clone()),
                        },
                        after_await: Vec::new(),
                    })
                }
                (false, false, false) => Err(syn::Error::new_spanned(
                    else_expr,
                    "internal error: else-if chain marked as containing await, but none found",
                )),
                _ => Err(syn::Error::new_spanned(
                    else_expr,
                    "#[corot] supports at most one await in an else/else-if chain",
                )),
            }
        }
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] else branch must be `else { ... }` or `else if ...`",
        )),
    }
}

fn else_suspend_before_await(es: &ElseSuspend) -> &[Stmt] {
    match es {
        ElseSuspend::FinalBlock { before_await }
        | ElseSuspend::ElseIfThen { before_await, .. } => before_await.as_slice(),
        ElseSuspend::ElseIfSkip { rest, .. } => else_suspend_before_await(rest),
        ElseSuspend::ElseIfCond { .. } => &[],
    }
}

fn emit_else_suspend(
    es: &ElseSuspend,
    go_wait: &proc_macro2::TokenStream,
    non_suspend: &proc_macro2::TokenStream,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match es {
        ElseSuspend::FinalBlock { before_await } => {
            let before = emit_stmts_rewrite_returns(before_await, ready_ok);
            quote! {
                #before
                #go_wait
            }
        }
        ElseSuspend::ElseIfThen {
            cond,
            before_await,
            rest_else,
        } => {
            let before = emit_stmts_rewrite_returns(before_await, ready_ok);
            let else_body = else_expr_tokens(rest_else, ready_ok);
            quote! {
                if #cond {
                    #before
                    #go_wait
                } else {
                    #else_body
                    #non_suspend
                }
            }
        }
        ElseSuspend::ElseIfSkip {
            cond,
            then_branch,
            rest,
        } => {
            let then_stmts = emit_stmts_rewrite_returns(&then_branch.stmts, ready_ok);
            let rest_code = emit_else_suspend(rest, go_wait, non_suspend, ready_ok);
            quote! {
                if #cond {
                    #then_stmts
                    #non_suspend
                } else {
                    #rest_code
                }
            }
        }
        ElseSuspend::ElseIfCond { .. } => {
            // Condition await: evaluate base + suspend (then/else run on resume).
            quote! { #go_wait }
        }
    }
}

fn extract_single_await_from_stmts(
    stmts: &[Stmt],
    err_ty: Option<&Type>,
) -> syn::Result<(Vec<Stmt>, PlainAwait, Vec<Stmt>)> {
    let mut before = Vec::new();
    let mut found: Option<PlainAwait> = None;
    let mut after = Vec::new();

    for stmt in stmts {
        if found.is_none() {
            if let Some(plain) = as_plain_await_let(stmt, err_ty, 0)? {
                found = Some(plain);
            } else if stmt_contains_await(stmt) {
                return Err(syn::Error::new_spanned(
                    stmt,
                    "#[corot] await inside a suspending block must be a typed let binding",
                ));
            } else {
                before.push(stmt.clone());
            }
        } else if stmt_contains_await(stmt) {
            return Err(syn::Error::new_spanned(
                stmt,
                "#[corot] only one await is supported inside this suspending block",
            ));
        } else {
            after.push(stmt.clone());
        }
    }

    let plain = found.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "internal error: expected await in branch",
        )
    })?;
    Ok((before, plain, after))
}

fn after_resume_stmts(ap: &AwaitPoint) -> Vec<&Stmt> {
    match &ap.kind {
        SuspendKind::Plain { after_resume } => after_resume.iter().collect(),
        SuspendKind::IfThen { after_await, .. }
        | SuspendKind::IfElse { after_await, .. }
        | SuspendKind::Loop { after_await, .. }
        | SuspendKind::While { after_await, .. }
        | SuspendKind::For { after_await, .. }
        | SuspendKind::MatchArm { after_await, .. }
        | SuspendKind::LetElseAwait { after_await, .. }
        | SuspendKind::LabeledBlock { after_await, .. }
        | SuspendKind::TryBlock { after_await, .. } => after_await.iter().collect(),
        SuspendKind::IfCondition { .. }
        | SuspendKind::IfLetScrutinee { .. }
        | SuspendKind::MatchScrutinee { .. }
        | SuspendKind::MatchGuard { .. } => Vec::new(),
    }
}

fn after_resume_escapes(kind: &SuspendKind) -> bool {
    matches!(kind, SuspendKind::Plain { .. })
}

fn pat_binds_after_resume(ap: &AwaitPoint) -> Vec<Binding> {
    match &ap.kind {
        SuspendKind::Plain { after_resume } => {
            // `let PAT = … else …` after resume introduces pattern bindings.
            after_resume
                .iter()
                .find_map(|stmt| {
                    let Stmt::Local(Local {
                        pat,
                        init: Some(init),
                        ..
                    }) = stmt
                    else {
                        return None;
                    };
                    let scrut_ty = resolve_scrut_ty(pat, &init.expr).ok()?;
                    bindings_from_pat(pat, &scrut_ty).ok()
                })
                .unwrap_or_default()
        }
        SuspendKind::IfLetScrutinee { pat, .. } => {
            // Bindings are scoped to then-branch only; join does not see them.
            let _ = pat;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn needs_join(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::IfThen { .. }
            | SuspendKind::IfElse { .. }
            | SuspendKind::Loop { .. }
            | SuspendKind::While { .. }
            | SuspendKind::For { .. }
            | SuspendKind::MatchArm { .. }
            | SuspendKind::MatchGuard { .. }
            | SuspendKind::LetElseAwait { .. }
            | SuspendKind::LabeledBlock { .. }
            | SuspendKind::TryBlock { .. }
    )
}

fn is_loop_kind(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::Loop { .. } | SuspendKind::While { .. } | SuspendKind::For { .. }
    )
}

fn is_match_join_kind(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::MatchArm { .. } | SuspendKind::MatchGuard { .. }
    )
}

fn waiting_iter_variant(index: usize) -> Ident {
    format_ident!("WaitingIter{}", index)
}

fn for_has_iter_await(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::For {
            iter_await_base: Some(_),
            ..
        }
    )
}

fn for_into_ty(kind: &SuspendKind) -> Option<&Type> {
    match kind {
        SuspendKind::For { into_ty, .. } => Some(into_ty),
        _ => None,
    }
}

fn after_if_variant(index: usize) -> Ident {
    format_ident!("AfterIf{}", index)
}

fn after_loop_variant(index: usize) -> Ident {
    format_ident!("AfterLoop{}", index)
}

fn after_match_variant(index: usize) -> Ident {
    format_ident!("AfterMatch{}", index)
}

fn loop_head_variant(index: usize) -> Ident {
    format_ident!("LoopHead{}", index)
}

fn join_variant(kind: &SuspendKind, index: usize) -> Ident {
    if is_loop_kind(kind) {
        after_loop_variant(index)
    } else if is_match_join_kind(kind) {
        after_match_variant(index)
    } else {
        after_if_variant(index)
    }
}

fn join_stmts_of(ap: &AwaitPoint) -> Option<&[Stmt]> {
    match &ap.kind {
        SuspendKind::IfThen { join_stmts, .. }
        | SuspendKind::IfElse { join_stmts, .. }
        | SuspendKind::Loop { join_stmts, .. }
        | SuspendKind::While { join_stmts, .. }
        | SuspendKind::For { join_stmts, .. }
        | SuspendKind::MatchArm { join_stmts, .. }
        | SuspendKind::MatchGuard { join_stmts, .. }
        | SuspendKind::LetElseAwait { join_stmts, .. }
        | SuspendKind::LabeledBlock { join_stmts, .. }
        | SuspendKind::TryBlock { join_stmts, .. } => Some(join_stmts.as_slice()),
        _ => None,
    }
}

fn effective_join_caps(ap: &AwaitPoint, join_caps: &[Binding]) -> Vec<Binding> {
    match &ap.kind {
        SuspendKind::LetElseAwait { pat_binds, .. } => {
            let mut caps = join_caps.to_vec();
            for b in pat_binds {
                upsert_binding(&mut caps, b.clone());
            }
            caps
        }
        SuspendKind::LabeledBlock {
            bind_name,
            bind_ty,
            ..
        }
        | SuspendKind::TryBlock {
            bind_name,
            bind_ty,
            ..
        } => {
            let mut caps = join_caps.to_vec();
            upsert_binding(
                &mut caps,
                Binding {
                    name: bind_name.clone(),
                    ty: bind_ty.clone(),
                    mutable: false,
                },
            );
            caps
        }
        SuspendKind::Loop {
            break_bind: Some((bind_name, bind_ty)),
            ..
        } => {
            let mut caps = join_caps.to_vec();
            upsert_binding(
                &mut caps,
                Binding {
                    name: bind_name.clone(),
                    ty: bind_ty.clone(),
                    mutable: false,
                },
            );
            caps
        }
        _ => join_caps.to_vec(),
    }
}

fn join_value_binding(ap: &AwaitPoint) -> Option<Binding> {
    match &ap.kind {
        SuspendKind::LabeledBlock {
            bind_name,
            bind_ty,
            ..
        }
        | SuspendKind::TryBlock {
            bind_name,
            bind_ty,
            ..
        } => Some(Binding {
            name: bind_name.clone(),
            ty: bind_ty.clone(),
            mutable: false,
        }),
        SuspendKind::Loop {
            break_bind: Some((bind_name, bind_ty)),
            ..
        } => Some(Binding {
            name: bind_name.clone(),
            ty: bind_ty.clone(),
            mutable: false,
        }),
        _ => None,
    }
}

fn loop_head_caps(ap: &AwaitPoint, join_caps: &[Binding]) -> Vec<Binding> {
    match &ap.kind {
        SuspendKind::For { into_ty, .. } => {
            let mut caps = join_caps.to_vec();
            caps.push(Binding {
                name: format_ident!("__iter"),
                ty: into_iter_ty(into_ty),
                mutable: true,
            });
            caps
        }
        _ => join_caps.to_vec(),
    }
}

fn assign_join_stmts(awaits: &mut [AwaitPoint], after_last: &mut Vec<Stmt>) {
    for i in 0..awaits.len() {
        if !needs_join(&awaits[i].kind) {
            continue;
        }
        let join = if i + 1 < awaits.len() {
            std::mem::take(&mut awaits[i + 1].before)
        } else {
            std::mem::take(after_last)
        };
        match &mut awaits[i].kind {
            SuspendKind::IfThen { join_stmts, .. }
            | SuspendKind::IfElse { join_stmts, .. }
            | SuspendKind::Loop { join_stmts, .. }
            | SuspendKind::While { join_stmts, .. }
            | SuspendKind::For { join_stmts, .. }
            | SuspendKind::MatchArm { join_stmts, .. }
            | SuspendKind::MatchGuard { join_stmts, .. }
            | SuspendKind::LetElseAwait { join_stmts, .. }
            | SuspendKind::LabeledBlock { join_stmts, .. }
            | SuspendKind::TryBlock { join_stmts, .. } => {
                *join_stmts = join;
            }
            _ => {}
        }
    }
}

fn field_tokens(b: &Binding) -> proc_macro2::TokenStream {
    let n = &b.name;
    let t = &b.ty;
    let skip = cfg!(feature = "serde") && is_skip_serde(&b.ty);
    if skip {
        quote! {
            #[serde(skip)]
            #n: #t
        }
    } else {
        quote! { #n: #t }
    }
}

fn cap_pat(b: &Binding) -> proc_macro2::TokenStream {
    let n = &b.name;
    if b.mutable {
        quote! { mut #n }
    } else {
        quote! { #n }
    }
}

fn else_expr_tokens(
    else_branch: &Option<Box<Expr>>,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match else_branch {
        Some(e) => emit_expr_rewrite_returns(e, ready_ok),
        None => quote! { {} },
    }
}

fn gen_goto_join(ap: &AwaitPoint, index: usize, join_caps: &[Binding]) -> proc_macro2::TokenStream {
    let var = join_variant(&ap.kind, index);
    let fields = join_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        *self = Self::#var {
            #(#fields,)*
        };
        continue 'step;
    }
}

fn gen_goto_loop_head(
    index: usize,
    ap: &AwaitPoint,
    join_caps: &[Binding],
) -> proc_macro2::TokenStream {
    let var = loop_head_variant(index);
    let fields = join_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    match &ap.kind {
        SuspendKind::For { .. } => quote! {
            *self = Self::#var {
                __iter,
                #(#fields,)*
            };
            continue 'step;
        },
        _ => quote! {
            *self = Self::#var {
                #(#fields,)*
            };
            continue 'step;
        },
    }
}

fn gen_enter_wait(
    ap: &AwaitPoint,
    waiting_var: &Ident,
    caps: &[Binding],
) -> proc_macro2::TokenStream {
    if ap.nested_child.is_some() {
        gen_go_nested(waiting_var, caps, &ap.base)
    } else {
        gen_go_waiting(waiting_var, caps, &ap.base)
    }
}

fn gen_go_nested(
    var: &Ident,
    caps: &[Binding],
    base: &Expr,
) -> proc_macro2::TokenStream {
    let cap_moves = caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        let __child = #base;
        *self = Self::#var {
            #(#cap_moves,)*
            __child,
        };
        continue 'step;
    }
}

fn gen_go_waiting(var: &Ident, caps: &[Binding], base: &Expr) -> proc_macro2::TokenStream {
    let cap_moves = caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        let _ = #base;
        *self = Self::#var {
            #(#cap_moves,)*
            __wait: ::core::option::Option::None,
        };
        break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
    }
}

fn gen_enter_await(
    index: usize,
    ap: &AwaitPoint,
    waiting_var: &Ident,
    caps: &[Binding],
    join_caps: &[Binding],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let before = emit_stmts_rewrite_returns(&ap.before, ready_ok);
    let go_wait = gen_enter_wait(ap, waiting_var, caps);
    // After* join caps may include expression values (loop/block break binds).
    // LoopHead / WaitingIter must not — those run before a value exists.
    let after_caps = effective_join_caps(ap, join_caps);
    let non_suspend = gen_goto_join(ap, index, &after_caps);

    match &ap.kind {
        SuspendKind::Plain { .. }
        | SuspendKind::IfCondition { .. }
        | SuspendKind::IfLetScrutinee { .. }
        | SuspendKind::MatchScrutinee { .. } => quote! {
            #before
            #go_wait
        },
        SuspendKind::Loop { .. } | SuspendKind::While { .. } => {
            let goto_head = gen_goto_loop_head(index, ap, join_caps);
            quote! {
                #before
                #goto_head
            }
        }
        SuspendKind::For {
            into_ty,
            iter_expr,
            iter_await_base,
            ..
        } => {
            let iter_ty = into_iter_ty(into_ty);
            let goto_head = gen_goto_loop_head(index, ap, join_caps);
            if let Some(base) = iter_await_base {
                let iter_var = waiting_iter_variant(index);
                let join_moves = join_caps.iter().map(|b| {
                    let n = &b.name;
                    quote! { #n }
                });
                quote! {
                    #before
                    let _ = #base;
                    *self = Self::#iter_var {
                        #(#join_moves,)*
                        __wait: ::core::option::Option::None,
                    };
                    break 'step ::core::result::Result::Ok(::core::task::Poll::Pending);
                }
            } else {
                let expr = iter_expr
                    .as_ref()
                    .expect("sync for must have iter_expr");
                quote! {
                    #before
                    let mut __iter: #iter_ty = ::core::iter::IntoIterator::into_iter(#expr);
                    #goto_head
                }
            }
        }
        SuspendKind::IfThen {
            cond,
            before_await,
            else_branch,
            ..
        } => {
            let before_await = emit_stmts_rewrite_returns(before_await, ready_ok);
            let else_body = else_expr_tokens(else_branch, ready_ok);
            quote! {
                #before
                if #cond {
                    #before_await
                    #go_wait
                } else {
                    #else_body
                    #non_suspend
                }
            }
        }
        SuspendKind::IfElse {
            cond,
            then_branch,
            else_suspend,
            ..
        } => {
            let then_stmts = emit_stmts_rewrite_returns(&then_branch.stmts, ready_ok);
            let else_body = emit_else_suspend(else_suspend, &go_wait, &non_suspend, ready_ok);
            quote! {
                #before
                if #cond {
                    #then_stmts
                    #non_suspend
                } else {
                    #else_body
                }
            }
        }
        SuspendKind::LetElseAwait {
            pat,
            init,
            before_await,
            ..
        } => {
            let before_await = emit_stmts_rewrite_returns(before_await, ready_ok);
            quote! {
                #before
                if let #pat = #init {
                    #non_suspend
                } else {
                    #before_await
                    #go_wait
                }
            }
        }
        SuspendKind::MatchArm {
            scrutinee,
            arms_before,
            sus_pat,
            sus_guard,
            before_await,
            arms_after,
            ..
        } => {
            let before_arms = arms_before
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend, ready_ok));
            let after_arms = arms_after
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend, ready_ok));
            let before_await = emit_stmts_rewrite_returns(before_await, ready_ok);
            let guard = match sus_guard {
                Some(g) => quote! { if #g },
                None => quote! {},
            };
            quote! {
                #before
                match #scrutinee {
                    #(#before_arms)*
                    #sus_pat #guard => {
                        #before_await
                        #go_wait
                    }
                    #(#after_arms)*
                }
            }
        }
        SuspendKind::MatchGuard {
            scrutinee,
            scrut_ty,
            arms_before,
            sus_pat,
            arms_after,
            ..
        } => {
            let before_arms = arms_before
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend, ready_ok));
            let after_arms = arms_after
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend, ready_ok));
            quote! {
                #before
                let __scrut: #scrut_ty = #scrutinee;
                match ::core::clone::Clone::clone(&__scrut) {
                    #(#before_arms)*
                    _ => {
                        match ::core::clone::Clone::clone(&__scrut) {
                            #sus_pat => {
                                #go_wait
                            }
                            #(#after_arms)*
                        }
                    }
                }
            }
        }
        SuspendKind::LabeledBlock {
            label,
            bind_name,
            bind_ty,
            before_await,
            ..
        } => {
            let before_await = rewrite_labeled_block_stmts(
                before_await,
                index,
                label,
                bind_name,
                bind_ty,
                &after_caps,
                ready_ok,
            );
            quote! {
                #before
                #before_await
                #go_wait
            }
        }
        SuspendKind::TryBlock {
            bind_name,
            bind_ty,
            before_await,
            ..
        } => {
            let before_await = rewrite_try_block_stmts(
                before_await,
                index,
                bind_name,
                bind_ty,
                &after_caps,
                ready_ok,
            );
            quote! {
                #before
                #before_await
                #go_wait
            }
        }
    }
}

fn emit_sync_arm_with_join(
    arm: &syn::Arm,
    goto_join: &proc_macro2::TokenStream,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let attrs = &arm.attrs;
    let pat = &arm.pat;
    let guard = match &arm.guard {
        Some((_, g)) => quote! { if #g },
        None => quote! {},
    };
    match arm.body.as_ref() {
        Expr::Block(b) => {
            let stmts = emit_stmts_rewrite_returns(&b.block.stmts, ready_ok);
            quote! {
                #(#attrs)*
                #pat #guard => {
                    #stmts
                    #goto_join
                }
            }
        }
        body => {
            let body = emit_expr_rewrite_returns(body, ready_ok);
            quote! {
                #(#attrs)*
                #pat #guard => {
                    #body;
                    #goto_join
                }
            }
        }
    }
}

fn emit_arm_body(
    arm: &syn::Arm,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let attrs = &arm.attrs;
    let pat = &arm.pat;
    let guard = match &arm.guard {
        Some((_, g)) => quote! { if #g },
        None => quote! {},
    };
    let body = emit_expr_rewrite_returns(&arm.body, ready_ok);
    quote! {
        #(#attrs)*
        #pat #guard => #body,
    }
}

fn gen_after_resume(
    index: usize,
    ap: &AwaitPoint,
    join_caps: &[Binding],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let try_bind = match (&ap.try_ok, &ap.kind) {
        (
            Some((name, ok_ty)),
            SuspendKind::TryBlock {
                bind_name,
                bind_ty,
                ..
            },
        ) => {
            let tmp = &ap.tmp;
            let join_caps = effective_join_caps(ap, join_caps);
            let var = after_if_variant(index);
            let fields = join_caps.iter().map(|b| {
                let n = &b.name;
                quote! { #n }
            });
            quote! {
                let #name: #ok_ty = match #tmp {
                    ::core::result::Result::Ok(v) => v,
                    ::core::result::Result::Err(e) => {
                        let #bind_name: #bind_ty = ::core::result::Result::Err(
                            ::core::convert::From::from(e),
                        );
                        *self = Self::#var {
                            #(#fields,)*
                        };
                        continue 'step;
                    }
                };
            }
        }
        (Some((name, ok_ty)), _) => {
            let tmp = &ap.tmp;
            quote! {
                let #name: #ok_ty = match #tmp {
                    ::core::result::Result::Ok(v) => v,
                    ::core::result::Result::Err(e) => {
                        *self = Self::Finished;
                        break 'step ::core::result::Result::Ok(
                            ::core::task::Poll::Ready(::core::result::Result::Err(
                                ::core::convert::From::from(e),
                            )),
                        );
                    }
                };
            }
        }
        (None, _) => quote! {},
    };

    let rest = match &ap.kind {
        SuspendKind::Plain { after_resume } => {
            emit_stmts_rewrite_returns(after_resume, ready_ok)
        }
        SuspendKind::IfCondition {
            resume_cond,
            then_branch,
            else_branch,
        } => {
            let then_stmts = emit_stmts_rewrite_returns(&then_branch.stmts, ready_ok);
            match else_branch {
                Some(e) => {
                    let else_body = emit_expr_rewrite_returns(e, ready_ok);
                    quote! {
                        if #resume_cond {
                            #then_stmts
                        } else #else_body;
                    }
                }
                None => quote! {
                    if #resume_cond {
                        #then_stmts
                    }
                },
            }
        }
        SuspendKind::IfLetScrutinee {
            pat,
            then_branch,
            else_branch,
        } => {
            let tmp = &ap.tmp;
            let then_stmts = emit_stmts_rewrite_returns(&then_branch.stmts, ready_ok);
            match else_branch {
                Some(e) => {
                    let else_body = emit_expr_rewrite_returns(e, ready_ok);
                    quote! {
                        if let #pat = #tmp {
                            #then_stmts
                        } else #else_body;
                    }
                }
                None => quote! {
                    if let #pat = #tmp {
                        #then_stmts
                    }
                },
            }
        }
        SuspendKind::MatchScrutinee { arms } => {
            let tmp = &ap.tmp;
            let arm_tokens = arms.iter().map(|a| emit_arm_body(a, ready_ok));
            quote! {
                match #tmp {
                    #(#arm_tokens)*
                }
            }
        }
        SuspendKind::IfThen { after_await, .. }
        | SuspendKind::MatchArm { after_await, .. } => {
            emit_stmts_rewrite_returns(after_await, ready_ok)
        }
        SuspendKind::IfElse {
            else_suspend,
            after_await,
            ..
        } => match else_suspend {
            ElseSuspend::ElseIfCond {
                then_branch,
                rest_else,
            } => {
                let tmp = &ap.tmp;
                let then_stmts = emit_stmts_rewrite_returns(&then_branch.stmts, ready_ok);
                match rest_else {
                    Some(e) => {
                        let else_body = emit_expr_rewrite_returns(e, ready_ok);
                        quote! {
                            if #tmp {
                                #then_stmts
                            } else #else_body;
                        }
                    }
                    None => quote! {
                        if #tmp {
                            #then_stmts
                        }
                    },
                }
            }
            _ => emit_stmts_rewrite_returns(after_await, ready_ok),
        },
        SuspendKind::LetElseAwait { after_await, .. } => {
            let parts = emit_stmts_rewrite_returns(after_await, ready_ok);
            let has_return = after_await.iter().any(stmt_has_return);
            if has_return {
                parts
            } else {
                quote! {
                    #parts
                    *self = Self::Finished;
                    break 'step ::core::result::Result::Ok(#ready_ok);
                }
            }
        }
        SuspendKind::MatchGuard {
            sus_pat,
            sus_body,
            arms_after,
            ..
        } => {
            let tmp = &ap.tmp;
            let sus_body = emit_expr_rewrite_returns(sus_body, ready_ok);
            let else_branch = if arms_after.is_empty() {
                quote! {}
            } else {
                let after_tokens = arms_after.iter().map(|a| emit_arm_body(a, ready_ok));
                quote! {
                    match __scrut {
                        #(#after_tokens)*
                    }
                }
            };
            quote! {
                if #tmp {
                    match __scrut {
                        #sus_pat => #sus_body,
                        _ => ::core::unreachable!("match guard passed but pattern failed"),
                    }
                } else {
                    #else_branch
                }
            }
        }
        SuspendKind::Loop {
            after_await,
            label,
            break_bind,
            ..
        } => rewrite_loop_body_stmts(
            index,
            after_await,
            join_caps,
            label.as_ref(),
            false,
            break_bind.as_ref(),
            ready_ok,
        ),
        SuspendKind::For {
            after_await,
            label,
            ..
        } => rewrite_loop_body_stmts(
            index,
            after_await,
            join_caps,
            label.as_ref(),
            true,
            None,
            ready_ok,
        ),
        SuspendKind::While {
            sync_cond: None,
            await_let_pat,
            after_await,
            label,
            ..
        } => {
            let tmp = &ap.tmp;
            let body = rewrite_loop_body_stmts(
                index,
                after_await,
                join_caps,
                label.as_ref(),
                false,
                None,
                ready_ok,
            );
            let goto_head = gen_goto_loop_head(index, ap, join_caps);
            let goto_after = gen_goto_join(ap, index, join_caps);
            match await_let_pat {
                Some(pat) => quote! {
                    if let #pat = #tmp {
                        #body
                        #goto_head
                    } else {
                        #goto_after
                    }
                },
                None => quote! {
                    if #tmp {
                        #body
                        #goto_head
                    } else {
                        #goto_after
                    }
                },
            }
        }
        SuspendKind::While {
            after_await,
            label,
            ..
        } => rewrite_loop_body_stmts(
            index,
            after_await,
            join_caps,
            label.as_ref(),
            false,
            None,
            ready_ok,
        ),
        SuspendKind::LabeledBlock {
            label,
            bind_name,
            bind_ty,
            is_stmt,
            after_await,
            ..
        } => {
            let join_caps = effective_join_caps(ap, join_caps);
            let (body_stmts, trailing) = split_block_trailing(after_await);
            let body = rewrite_labeled_block_stmts(
                &body_stmts,
                index,
                label,
                bind_name,
                bind_ty,
                &join_caps,
                ready_ok,
            );
            let value = match trailing {
                Some(e) => rewrite_labeled_block_expr(
                    e,
                    &LabeledBlockRewriteCtx {
                        index,
                        label,
                        bind_name,
                        bind_ty,
                        join_caps: &join_caps,
                        ready_ok,
                    },
                    false,
                ),
                None if *is_stmt || is_unit_ty(bind_ty) => quote! { () },
                // Unreachable if every path `break`s; still type-checked by rustc.
                None => quote! {
                    ::core::unreachable!(
                        "#[corot] labeled block exited without a value"
                    )
                },
            };
            quote! {
                #body
                let #bind_name: #bind_ty = #value;
            }
        }
        SuspendKind::TryBlock {
            bind_name,
            bind_ty,
            after_await,
            ..
        } => {
            let join_caps = effective_join_caps(ap, join_caps);
            let (body_stmts, trailing) = split_block_trailing(after_await);
            let body = rewrite_try_block_stmts(
                &body_stmts,
                index,
                bind_name,
                bind_ty,
                &join_caps,
                ready_ok,
            );
            let value = match trailing {
                Some(e) => rewrite_try_block_expr(
                    e,
                    &TryBlockRewriteCtx {
                        index,
                        bind_name,
                        bind_ty,
                        join_caps: &join_caps,
                        ready_ok,
                    },
                ),
                None => quote! { () },
            };
            quote! {
                #body
                let #bind_name: #bind_ty = ::core::result::Result::Ok(#value);
            }
        }
    };

    quote! {
        #try_bind
        #rest
    }
}

fn split_block_trailing(stmts: &[Stmt]) -> (Vec<Stmt>, Option<&Expr>) {
    match stmts.split_last() {
        Some((Stmt::Expr(e, None), rest))
            if !matches!(
                e,
                Expr::Break(_) | Expr::Return(_) | Expr::Continue(_)
            ) =>
        {
            (rest.to_vec(), Some(e))
        }
        _ => (stmts.to_vec(), None),
    }
}

struct LabeledBlockRewriteCtx<'a> {
    index: usize,
    label: &'a Ident,
    bind_name: &'a Ident,
    bind_ty: &'a Type,
    join_caps: &'a [Binding],
    ready_ok: &'a proc_macro2::TokenStream,
}

fn rewrite_labeled_block_stmts(
    stmts: &[Stmt],
    index: usize,
    label: &Ident,
    bind_name: &Ident,
    bind_ty: &Type,
    join_caps: &[Binding],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let ctx = LabeledBlockRewriteCtx {
        index,
        label,
        bind_name,
        bind_ty,
        join_caps,
        ready_ok,
    };
    let parts = stmts
        .iter()
        .map(|stmt| rewrite_labeled_block_stmt(stmt, &ctx, false));
    quote! { #(#parts)* }
}

fn emit_labeled_block_break(
    ctx: &LabeledBlockRewriteCtx<'_>,
    value: Option<&Expr>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    let bind_name = ctx.bind_name;
    let bind_ty = ctx.bind_ty;
    let val = match value {
        Some(e) => rewrite_labeled_block_expr(e, ctx, in_nested),
        None => quote! { () },
    };
    let var = after_if_variant(ctx.index);
    let fields = ctx.join_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        let #bind_name: #bind_ty = #val;
        *self = Self::#var {
            #(#fields,)*
        };
        continue 'step;
    }
}

fn rewrite_labeled_block_break(
    brk: &syn::ExprBreak,
    ctx: &LabeledBlockRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match &brk.label {
        Some(life) if life.ident == *ctx.label => {
            emit_labeled_block_break(ctx, brk.expr.as_deref(), in_nested)
        }
        None if !in_nested => quote! {
            ::core::compile_error!("#[corot] unlabeled `break` is not valid in a labeled block")
        },
        None => quote! { break },
        Some(life) if in_nested => match &brk.expr {
            Some(e) => quote! { break #life #e },
            None => quote! { break #life },
        },
        Some(life) => {
            let msg = format!(
                "#[corot] `break '{0}` does not match this labeled block's label",
                life.ident
            );
            quote! { ::core::compile_error!(#msg) }
        }
    }
}

fn rewrite_labeled_block_continue(
    cont: &syn::ExprContinue,
    ctx: &LabeledBlockRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match &cont.label {
        Some(life) if life.ident == *ctx.label => quote! {
            ::core::compile_error!("#[corot] `continue` cannot target a labeled block")
        },
        None if !in_nested => quote! {
            ::core::compile_error!("#[corot] unlabeled `continue` is not valid in a labeled block")
        },
        None => quote! { continue },
        Some(life) if in_nested => quote! { continue #life },
        Some(life) => {
            let msg = format!(
                "#[corot] `continue '{0}` does not match a loop label in this labeled block",
                life.ident
            );
            quote! { ::core::compile_error!(#msg) }
        }
    }
}

fn rewrite_labeled_block_stmt(
    stmt: &Stmt,
    ctx: &LabeledBlockRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Expr(Expr::Return(ret), _) => {
            emit_return_finish(ret.expr.as_deref(), ctx.ready_ok)
        }
        Stmt::Expr(Expr::Break(brk), _) => rewrite_labeled_block_break(brk, ctx, in_nested),
        Stmt::Expr(Expr::Continue(cont), _) => {
            rewrite_labeled_block_continue(cont, ctx, in_nested)
        }
        Stmt::Expr(expr, semi) => {
            let e = rewrite_labeled_block_expr(expr, ctx, in_nested);
            match semi {
                Some(_) => quote! { #e; },
                None => quote! { #e },
            }
        }
        other => emit_stmt_rewrite_returns(other, ctx.ready_ok),
    }
}

fn rewrite_labeled_block_expr(
    expr: &Expr,
    ctx: &LabeledBlockRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match expr {
        Expr::Return(ret) => emit_return_finish(ret.expr.as_deref(), ctx.ready_ok),
        Expr::Break(brk) => rewrite_labeled_block_break(brk, ctx, in_nested),
        Expr::Continue(cont) => rewrite_labeled_block_continue(cont, ctx, in_nested),
        Expr::Block(b) => {
            let parts = b
                .block
                .stmts
                .iter()
                .map(|s| rewrite_labeled_block_stmt(s, ctx, in_nested));
            let label = &b.label;
            quote! { #label { #(#parts)* } }
        }
        Expr::If(expr_if) => {
            let cond = &expr_if.cond;
            let then_parts = expr_if
                .then_branch
                .stmts
                .iter()
                .map(|s| rewrite_labeled_block_stmt(s, ctx, in_nested));
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #(#then_parts)*
                    }
                },
                Some((_, else_expr)) => {
                    let else_body = rewrite_labeled_block_expr(else_expr, ctx, in_nested);
                    quote! {
                        if #cond {
                            #(#then_parts)*
                        } else #else_body
                    }
                }
            }
        }
        Expr::Match(m) => {
            let scrut = &m.expr;
            let arms = m.arms.iter().map(|arm| {
                let attrs = &arm.attrs;
                let pat = &arm.pat;
                let guard = match &arm.guard {
                    Some((_, g)) => quote! { if #g },
                    None => quote! {},
                };
                let body = rewrite_labeled_block_expr(&arm.body, ctx, in_nested);
                quote! {
                    #(#attrs)*
                    #pat #guard => #body,
                }
            });
            quote! {
                match #scrut {
                    #(#arms)*
                }
            }
        }
        Expr::Loop(l) => {
            let body = l
                .body
                .stmts
                .iter()
                .map(|s| rewrite_labeled_block_stmt(s, ctx, true));
            let label = &l.label;
            quote! {
                #label loop {
                    #(#body)*
                }
            }
        }
        Expr::While(w) => {
            let cond = &w.cond;
            let body = w
                .body
                .stmts
                .iter()
                .map(|s| rewrite_labeled_block_stmt(s, ctx, true));
            let label = &w.label;
            quote! {
                #label while #cond {
                    #(#body)*
                }
            }
        }
        Expr::ForLoop(f) => {
            let pat = &f.pat;
            let iter = &f.expr;
            let body = f
                .body
                .stmts
                .iter()
                .map(|s| rewrite_labeled_block_stmt(s, ctx, true));
            let label = &f.label;
            quote! {
                #label for #pat in #iter {
                    #(#body)*
                }
            }
        }
        Expr::Paren(p) => {
            let inner = rewrite_labeled_block_expr(&p.expr, ctx, in_nested);
            quote! { (#inner) }
        }
        Expr::Group(g) => rewrite_labeled_block_expr(&g.expr, ctx, in_nested),
        other => emit_expr_rewrite_returns(other, ctx.ready_ok),
    }
}

struct TryBlockRewriteCtx<'a> {
    index: usize,
    bind_name: &'a Ident,
    bind_ty: &'a Type,
    join_caps: &'a [Binding],
    ready_ok: &'a proc_macro2::TokenStream,
}

fn rewrite_try_block_stmts(
    stmts: &[Stmt],
    index: usize,
    bind_name: &Ident,
    bind_ty: &Type,
    join_caps: &[Binding],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let ctx = TryBlockRewriteCtx {
        index,
        bind_name,
        bind_ty,
        join_caps,
        ready_ok,
    };
    let parts = stmts
        .iter()
        .map(|stmt| rewrite_try_block_stmt(stmt, &ctx));
    quote! { #(#parts)* }
}

fn emit_try_block_err_join(
    ctx: &TryBlockRewriteCtx<'_>,
    err_expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let bind_name = ctx.bind_name;
    let bind_ty = ctx.bind_ty;
    let var = after_if_variant(ctx.index);
    let fields = ctx.join_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        let #bind_name: #bind_ty = ::core::result::Result::Err(
            ::core::convert::From::from(#err_expr),
        );
        *self = Self::#var {
            #(#fields,)*
        };
        continue 'step;
    }
}

fn rewrite_try_block_stmt(
    stmt: &Stmt,
    ctx: &TryBlockRewriteCtx<'_>,
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Local(local) => {
            let attrs = &local.attrs;
            let pat = &local.pat;
            if let Some(init) = &local.init {
                let expr = rewrite_try_block_expr(&init.expr, ctx);
                if let Some((_, diverge)) = &init.diverge {
                    let else_body = rewrite_try_block_expr(diverge, ctx);
                    return quote! {
                        #(#attrs)*
                        let #pat = #expr else #else_body;
                    };
                }
                return quote! {
                    #(#attrs)*
                    let #pat = #expr;
                };
            }
            quote! { #stmt }
        }
        Stmt::Expr(Expr::Return(ret), _) => {
            emit_return_finish(ret.expr.as_deref(), ctx.ready_ok)
        }
        Stmt::Expr(expr, semi) => {
            let e = rewrite_try_block_expr(expr, ctx);
            match semi {
                Some(_) => quote! { #e; },
                None => quote! { #e },
            }
        }
        other => emit_stmt_rewrite_returns(other, ctx.ready_ok),
    }
}

fn rewrite_try_block_expr(
    expr: &Expr,
    ctx: &TryBlockRewriteCtx<'_>,
) -> proc_macro2::TokenStream {
    match expr {
        Expr::Try(t) => {
            let inner = rewrite_try_block_expr(&t.expr, ctx);
            let on_err = emit_try_block_err_join(ctx, quote! { __e });
            quote! {
                match #inner {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => {
                        #on_err
                    }
                }
            }
        }
        Expr::Return(ret) => emit_return_finish(ret.expr.as_deref(), ctx.ready_ok),
        Expr::TryBlock(tb) => emit_sync_try_block(&tb.block.stmts, ctx.ready_ok),
        Expr::Block(b) => {
            let parts = b
                .block
                .stmts
                .iter()
                .map(|s| rewrite_try_block_stmt(s, ctx));
            let label = &b.label;
            quote! { #label { #(#parts)* } }
        }
        Expr::If(expr_if) => {
            let cond = rewrite_try_block_expr(&expr_if.cond, ctx);
            let then_parts = expr_if
                .then_branch
                .stmts
                .iter()
                .map(|s| rewrite_try_block_stmt(s, ctx));
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #(#then_parts)*
                    }
                },
                Some((_, else_expr)) => {
                    let else_body = rewrite_try_block_expr(else_expr, ctx);
                    quote! {
                        if #cond {
                            #(#then_parts)*
                        } else #else_body
                    }
                }
            }
        }
        Expr::Match(m) => {
            let scrut = rewrite_try_block_expr(&m.expr, ctx);
            let arms = m.arms.iter().map(|arm| {
                let attrs = &arm.attrs;
                let pat = &arm.pat;
                let guard = match &arm.guard {
                    Some((_, g)) => {
                        let g = rewrite_try_block_expr(g, ctx);
                        quote! { if #g }
                    }
                    None => quote! {},
                };
                let body = rewrite_try_block_expr(&arm.body, ctx);
                quote! {
                    #(#attrs)*
                    #pat #guard => #body,
                }
            });
            quote! {
                match #scrut {
                    #(#arms)*
                }
            }
        }
        Expr::Loop(l) => {
            let body = l
                .body
                .stmts
                .iter()
                .map(|s| rewrite_try_block_stmt(s, ctx));
            let label = &l.label;
            quote! {
                #label loop {
                    #(#body)*
                }
            }
        }
        Expr::While(w) => {
            let cond = rewrite_try_block_expr(&w.cond, ctx);
            let body = w
                .body
                .stmts
                .iter()
                .map(|s| rewrite_try_block_stmt(s, ctx));
            let label = &w.label;
            quote! {
                #label while #cond {
                    #(#body)*
                }
            }
        }
        Expr::ForLoop(f) => {
            let pat = &f.pat;
            let iter = rewrite_try_block_expr(&f.expr, ctx);
            let body = f
                .body
                .stmts
                .iter()
                .map(|s| rewrite_try_block_stmt(s, ctx));
            let label = &f.label;
            quote! {
                #label for #pat in #iter {
                    #(#body)*
                }
            }
        }
        Expr::Call(c) => {
            let func = rewrite_try_block_expr(&c.func, ctx);
            let args = c.args.iter().map(|a| rewrite_try_block_expr(a, ctx));
            quote! { #func(#(#args),*) }
        }
        Expr::MethodCall(m) => {
            let receiver = rewrite_try_block_expr(&m.receiver, ctx);
            let method = &m.method;
            let turbofish = &m.turbofish;
            let args = m.args.iter().map(|a| rewrite_try_block_expr(a, ctx));
            quote! { #receiver.#method #turbofish (#(#args),*) }
        }
        Expr::Binary(b) => {
            let left = rewrite_try_block_expr(&b.left, ctx);
            let op = &b.op;
            let right = rewrite_try_block_expr(&b.right, ctx);
            quote! { #left #op #right }
        }
        Expr::Unary(u) => {
            let op = &u.op;
            let expr = rewrite_try_block_expr(&u.expr, ctx);
            quote! { #op #expr }
        }
        Expr::Paren(p) => {
            let inner = rewrite_try_block_expr(&p.expr, ctx);
            quote! { (#inner) }
        }
        Expr::Group(g) => rewrite_try_block_expr(&g.expr, ctx),
        Expr::Closure(_) | Expr::Async(_) => quote! { #expr },
        other => emit_expr_rewrite_returns(other, ctx.ready_ok),
    }
}

fn rewrite_loop_body_stmts(
    index: usize,
    stmts: &[Stmt],
    join_caps: &[Binding],
    label: Option<&Ident>,
    is_for: bool,
    break_bind: Option<&(Ident, Type)>,
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let ctx = LoopRewriteCtx {
        index,
        join_caps,
        label,
        is_for,
        break_bind,
        ready_ok,
    };
    let parts = stmts
        .iter()
        .map(|stmt| rewrite_loop_stmt(stmt, &ctx, false));
    quote! { #(#parts)* }
}

struct LoopRewriteCtx<'a> {
    index: usize,
    join_caps: &'a [Binding],
    label: Option<&'a Ident>,
    is_for: bool,
    break_bind: Option<&'a (Ident, Type)>,
    ready_ok: &'a proc_macro2::TokenStream,
}

fn emit_loop_break(
    ctx: &LoopRewriteCtx<'_>,
    value: Option<&Expr>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    let mut after_caps = ctx.join_caps.to_vec();
    let assign = match (ctx.break_bind, value) {
        (Some((name, ty)), Some(e)) => {
            let val = rewrite_loop_expr(e, ctx, in_nested);
            upsert_binding(
                &mut after_caps,
                Binding {
                    name: name.clone(),
                    ty: ty.clone(),
                    mutable: false,
                },
            );
            quote! { let #name: #ty = #val; }
        }
        (Some((name, ty)), None) if is_unit_ty(ty) => {
            upsert_binding(
                &mut after_caps,
                Binding {
                    name: name.clone(),
                    ty: ty.clone(),
                    mutable: false,
                },
            );
            quote! { let #name: #ty = (); }
        }
        (Some(_), None) => {
            return quote! {
                ::core::compile_error!(
                    "#[corot] `break` in a loop-as-expression requires a value"
                )
            };
        }
        (None, Some(_)) => {
            return quote! {
                ::core::compile_error!(
                    "#[corot] `break` with a value requires `let name: T = loop { ... }`"
                )
            };
        }
        (None, None) => quote! {},
    };
    let var = after_loop_variant(ctx.index);
    let fields = after_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    quote! {
        #assign
        *self = Self::#var {
            #(#fields,)*
        };
        continue 'step;
    }
}

fn emit_loop_continue(ctx: &LoopRewriteCtx<'_>) -> proc_macro2::TokenStream {
    let var = loop_head_variant(ctx.index);
    let fields = ctx.join_caps.iter().map(|b| {
        let n = &b.name;
        quote! { #n }
    });
    if ctx.is_for {
        quote! {
            *self = Self::#var {
                __iter,
                #(#fields,)*
            };
            continue 'step;
        }
    } else {
        quote! {
            *self = Self::#var {
                #(#fields,)*
            };
            continue 'step;
        }
    }
}

fn label_matches(ctx: &LoopRewriteCtx<'_>, life: &syn::Lifetime) -> bool {
    ctx.label.is_some_and(|l| l == &life.ident)
}

fn rewrite_break_expr(
    brk: &syn::ExprBreak,
    ctx: &LoopRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    let value = brk.expr.as_deref();
    match &brk.label {
        None if !in_nested => emit_loop_break(ctx, value, in_nested),
        None => match value {
            Some(e) => quote! { break #e },
            None => quote! { break },
        },
        Some(life) if label_matches(ctx, life) => emit_loop_break(ctx, value, in_nested),
        Some(life) if in_nested => match value {
            Some(e) => quote! { break #life #e },
            None => quote! { break #life },
        },
        Some(life) => {
            let msg = format!(
                "#[corot] `break '{0}` does not match this suspending loop's label",
                life.ident
            );
            quote! { ::core::compile_error!(#msg) }
        }
    }
}

fn rewrite_continue_expr(
    cont: &syn::ExprContinue,
    ctx: &LoopRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match &cont.label {
        None if !in_nested => emit_loop_continue(ctx),
        None => quote! { continue },
        Some(life) if label_matches(ctx, life) => emit_loop_continue(ctx),
        Some(life) if in_nested => quote! { continue #life },
        Some(life) => {
            let msg = format!(
                "#[corot] `continue '{0}` does not match this suspending loop's label",
                life.ident
            );
            quote! { ::core::compile_error!(#msg) }
        }
    }
}

fn rewrite_loop_stmt(
    stmt: &Stmt,
    ctx: &LoopRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Expr(Expr::Return(ret), _) => {
            emit_return_finish(ret.expr.as_deref(), ctx.ready_ok)
        }
        Stmt::Expr(Expr::Break(brk), _) => rewrite_break_expr(brk, ctx, in_nested),
        Stmt::Expr(Expr::Continue(cont), _) => rewrite_continue_expr(cont, ctx, in_nested),
        Stmt::Expr(expr, semi) => {
            let e = rewrite_loop_expr(expr, ctx, in_nested);
            match semi {
                Some(_) => quote! { #e; },
                None => quote! { #e },
            }
        }
        other => emit_stmt_rewrite_returns(other, ctx.ready_ok),
    }
}

fn rewrite_loop_expr(
    expr: &Expr,
    ctx: &LoopRewriteCtx<'_>,
    in_nested: bool,
) -> proc_macro2::TokenStream {
    match expr {
        Expr::Return(ret) => emit_return_finish(ret.expr.as_deref(), ctx.ready_ok),
        Expr::Break(brk) => rewrite_break_expr(brk, ctx, in_nested),
        Expr::Continue(cont) => rewrite_continue_expr(cont, ctx, in_nested),
        Expr::Block(b) => {
            let parts = b
                .block
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, ctx, in_nested));
            quote! {{ #(#parts)* }}
        }
        Expr::If(expr_if) => {
            let cond = &expr_if.cond;
            let then_parts = expr_if
                .then_branch
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, ctx, in_nested));
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #(#then_parts)*
                    }
                },
                Some((_, else_expr)) => {
                    let else_body = rewrite_loop_expr(else_expr, ctx, in_nested);
                    quote! {
                        if #cond {
                            #(#then_parts)*
                        } else #else_body
                    }
                }
            }
        }
        Expr::Match(m) => {
            let scrut = &m.expr;
            let arms = m.arms.iter().map(|arm| {
                let attrs = &arm.attrs;
                let pat = &arm.pat;
                let guard = match &arm.guard {
                    Some((_, g)) => quote! { if #g },
                    None => quote! {},
                };
                let body = rewrite_loop_expr(&arm.body, ctx, in_nested);
                quote! {
                    #(#attrs)*
                    #pat #guard => #body,
                }
            });
            quote! {
                match #scrut {
                    #(#arms)*
                }
            }
        }
        // Nested sync loops: unlabeled break/continue stay native; labeled ones
        // targeting the suspending loop still rewrite.
        Expr::Loop(l) => {
            let body = l
                .body
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, ctx, true));
            let label = &l.label;
            quote! {
                #label loop {
                    #(#body)*
                }
            }
        }
        Expr::While(w) => {
            let cond = &w.cond;
            let body = w
                .body
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, ctx, true));
            let label = &w.label;
            quote! {
                #label while #cond {
                    #(#body)*
                }
            }
        }
        Expr::ForLoop(f) => {
            let pat = &f.pat;
            let iter = &f.expr;
            let body = f
                .body
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, ctx, true));
            let label = &f.label;
            quote! {
                #label for #pat in #iter {
                    #(#body)*
                }
            }
        }
        Expr::Paren(p) => {
            let inner = rewrite_loop_expr(&p.expr, ctx, in_nested);
            quote! { (#inner) }
        }
        Expr::Group(g) => rewrite_loop_expr(&g.expr, ctx, in_nested),
        other => emit_expr_rewrite_returns(other, ctx.ready_ok),
    }
}

/// Continuation after await `completed_i` is fully done (including join).
fn gen_join_tail(
    completed_i: usize,
    awaits: &[AwaitPoint],
    captures_at_await: &[Vec<Binding>],
    join_caps_at: &[Vec<Binding>],
    after_last: &[Stmt],
    ready_ok: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if completed_i + 1 >= awaits.len() {
        emit_completion_stmts(after_last, ready_ok)
    } else {
        let next_i = completed_i + 1;
        let next = &awaits[next_i];
        let next_var = waiting_variant(&next.name);
        let next_caps = &captures_at_await[next_i];
        gen_enter_await(
            next_i,
            next,
            &next_var,
            next_caps,
            &join_caps_at[next_i],
            ready_ok,
        )
    }
}

fn parse_fn_output(output: &syn::ReturnType) -> syn::Result<(Type, Option<Type>)> {
    match output {
        syn::ReturnType::Default => Ok((syn::parse_quote!(()), None)),
        syn::ReturnType::Type(_, ty) => {
            if is_unit_ty(ty) {
                return Ok((syn::parse_quote!(()), None));
            }
            if let Some(err) = result_err_ty(ty) {
                // Only `Result<(), E>` for now (Ok payload must be unit).
                if result_ok_ty(ty).is_some_and(|ok| is_unit_ty(&ok)) {
                    return Ok(((**ty).clone(), Some(err)));
                }
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[corot] with `await?` currently supports `Result<(), E>` return types only",
                ));
            }
            Err(syn::Error::new_spanned(
                ty,
                "#[corot] return type must be `()` or `Result<(), E>`",
            ))
        }
    }
}

fn is_unit_ty(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

fn ready_ok_tokens(output_ty: &Type) -> proc_macro2::TokenStream {
    if is_unit_ty(output_ty) {
        quote! { ::core::task::Poll::Ready(()) }
    } else {
        quote! { ::core::task::Poll::Ready(::core::result::Result::Ok(())) }
    }
}

fn contains_await(expr: &Expr) -> bool {
    match expr {
        Expr::Await(_) => true,
        Expr::Array(e) => e.elems.iter().any(contains_await),
        Expr::Assign(e) => contains_await(&e.left) || contains_await(&e.right),
        Expr::Async(_) | Expr::Closure(_) | Expr::Const(_) => false,
        Expr::Binary(e) => contains_await(&e.left) || contains_await(&e.right),
        Expr::Block(e) => e.block.stmts.iter().any(stmt_contains_await),
        Expr::Break(e) => e.expr.as_ref().is_some_and(|x| contains_await(x)),
        Expr::Call(e) => contains_await(&e.func) || e.args.iter().any(contains_await),
        Expr::Cast(e) => contains_await(&e.expr),
        Expr::Field(e) => contains_await(&e.base),
        Expr::ForLoop(e) => contains_await(&e.expr) || e.body.stmts.iter().any(stmt_contains_await),
        Expr::Group(e) => contains_await(&e.expr),
        Expr::If(e) => {
            contains_await(&e.cond)
                || e.then_branch.stmts.iter().any(stmt_contains_await)
                || e.else_branch
                    .as_ref()
                    .is_some_and(|(_, x)| contains_await(x))
        }
        Expr::Index(e) => contains_await(&e.expr) || contains_await(&e.index),
        Expr::Let(e) => contains_await(&e.expr),
        Expr::Loop(e) => e.body.stmts.iter().any(stmt_contains_await),
        Expr::Match(e) => {
            contains_await(&e.expr)
                || e.arms.iter().any(|a| {
                    contains_await(&a.body)
                        || a.guard
                            .as_ref()
                            .is_some_and(|(_, g)| contains_await(g))
                })
        }
        Expr::MethodCall(e) => contains_await(&e.receiver) || e.args.iter().any(contains_await),
        Expr::Paren(e) => contains_await(&e.expr),
        Expr::Range(e) => {
            e.start.as_ref().is_some_and(|x| contains_await(x))
                || e.end.as_ref().is_some_and(|x| contains_await(x))
        }
        Expr::Reference(e) => contains_await(&e.expr),
        Expr::Repeat(e) => contains_await(&e.expr) || contains_await(&e.len),
        Expr::Return(e) => e.expr.as_ref().is_some_and(|x| contains_await(x)),
        Expr::Struct(e) => e.fields.iter().any(|f| contains_await(&f.expr)),
        Expr::Try(e) => contains_await(&e.expr),
        Expr::TryBlock(e) => e.block.stmts.iter().any(stmt_contains_await),
        Expr::Tuple(e) => e.elems.iter().any(contains_await),
        Expr::Unary(e) => contains_await(&e.expr),
        Expr::Unsafe(e) => e.block.stmts.iter().any(stmt_contains_await),
        Expr::While(e) => contains_await(&e.cond) || e.body.stmts.iter().any(stmt_contains_await),
        Expr::Yield(e) => e.expr.as_ref().is_some_and(|x| contains_await(x)),
        _ => false,
    }
}

fn stmt_contains_await(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Local(Local {
            init: Some(init), ..
        }) => {
            contains_await(&init.expr)
                || init
                    .diverge
                    .as_ref()
                    .is_some_and(|(_, e)| contains_await(e))
        }
        Stmt::Expr(expr, _) => contains_await(expr),
        Stmt::Macro(_) => false,
        Stmt::Item(_) => false,
        _ => false,
    }
}

/// Replace the first `.await` with `replacement`, returning the await base expr.
fn replace_first_await(expr: &mut Expr, replacement: Expr) -> syn::Result<Expr> {
    match expr {
        Expr::Await(expr_await) => {
            let base = expr_await.base.as_ref().clone();
            *expr = replacement;
            Ok(base)
        }
        Expr::Array(e) => replace_in_slice(&mut e.elems, replacement),
        Expr::Assign(e) => replace_first_await_bin(&mut e.left, &mut e.right, replacement),
        Expr::Binary(e) => replace_first_await_bin(&mut e.left, &mut e.right, replacement),
        Expr::Call(e) => {
            if contains_await(&e.func) {
                replace_first_await(&mut e.func, replacement)
            } else {
                replace_in_slice(&mut e.args, replacement)
            }
        }
        Expr::Cast(e) => replace_first_await(&mut e.expr, replacement),
        Expr::Field(e) => replace_first_await(&mut e.base, replacement),
        Expr::Group(e) => replace_first_await(&mut e.expr, replacement),
        Expr::Index(e) => {
            if contains_await(&e.expr) {
                replace_first_await(&mut e.expr, replacement)
            } else {
                replace_first_await(&mut e.index, replacement)
            }
        }
        Expr::MethodCall(e) => {
            if contains_await(&e.receiver) {
                replace_first_await(&mut e.receiver, replacement)
            } else {
                replace_in_slice(&mut e.args, replacement)
            }
        }
        Expr::Paren(e) => replace_first_await(&mut e.expr, replacement),
        Expr::Reference(e) => replace_first_await(&mut e.expr, replacement),
        Expr::Try(e) => replace_first_await(&mut e.expr, replacement),
        Expr::Tuple(e) => replace_in_slice(&mut e.elems, replacement),
        Expr::Unary(e) => replace_first_await(&mut e.expr, replacement),
        other => Err(syn::Error::new_spanned(
            other,
            "#[corot] cannot split await inside this expression yet",
        )),
    }
}

fn replace_first_await_bin(
    left: &mut Expr,
    right: &mut Expr,
    replacement: Expr,
) -> syn::Result<Expr> {
    if contains_await(left) {
        replace_first_await(left, replacement)
    } else {
        replace_first_await(right, replacement)
    }
}

fn replace_in_slice(
    exprs: &mut syn::punctuated::Punctuated<Expr, syn::Token![,]>,
    replacement: Expr,
) -> syn::Result<Expr> {
    for expr in exprs.iter_mut() {
        if contains_await(expr) {
            return replace_first_await(expr, replacement);
        }
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "internal error: await not found in list",
    ))
}

fn ident_expr(ident: &Ident) -> Expr {
    Expr::Path(ExprPath {
        attrs: Vec::new(),
        qself: None,
        path: Path::from(ident.clone()),
    })
}

fn typed_let_binding(stmt: &Stmt) -> Option<Binding> {
    let Stmt::Local(Local { pat, .. }) = stmt else {
        return None;
    };
    let Pat::Type(PatType { pat, ty, .. }) = pat else {
        return None;
    };
    let (name, mutable) = match pat.as_ref() {
        Pat::Ident(PatIdent {
            ident, mutability, ..
        }) => (ident.clone(), mutability.is_some()),
        _ => return None,
    };
    Some(Binding {
        name,
        ty: ty.as_ref().clone(),
        mutable,
    })
}

fn pat_ident(pat: &Pat) -> syn::Result<Ident> {
    match pat {
        Pat::Ident(PatIdent { ident, .. }) => Ok(ident.clone()),
        other => Err(syn::Error::new_spanned(
            other,
            "only simple ident patterns are supported",
        )),
    }
}

fn pat_ident_or_discard(pat: &Pat) -> syn::Result<Ident> {
    match pat {
        Pat::Ident(PatIdent { ident, .. }) => Ok(ident.clone()),
        Pat::Wild(_) => Ok(format_ident!("_unit")),
        other => Err(syn::Error::new_spanned(
            other,
            "await bindings must be `let name: Type = …` or `let _: Type = …`",
        )),
    }
}

fn upsert_binding(live: &mut Vec<Binding>, b: Binding) {
    if let Some(existing) = live.iter_mut().find(|x| x.name == b.name) {
        existing.ty = b.ty;
        existing.mutable |= b.mutable;
    } else {
        live.push(b);
    }
}

/// True if the type path's last segment is `SkipSerde` (e.g. `SkipSerde<T>`,
/// `corot_rs::SkipSerde<T>`). Proc macros cannot check trait impls; this marker
/// is the opt-out for serde / rehydration.
fn is_skip_serde(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "SkipSerde")
}

fn collect_skip_bindings(captures_at_await: &[Vec<Binding>]) -> Vec<Binding> {
    let mut skips = Vec::new();
    for caps in captures_at_await {
        for b in caps {
            if is_skip_serde(&b.ty) {
                upsert_binding(&mut skips, b.clone());
            }
        }
    }
    skips
}

fn collect_all_bindings(captures_at_await: &[Vec<Binding>]) -> Vec<Binding> {
    let mut all = Vec::new();
    for caps in captures_at_await {
        for b in caps {
            upsert_binding(&mut all, b.clone());
        }
    }
    all
}

fn rehydration_guard(
    rehyd_name: &Ident,
    var: &Ident,
    caps: &[Binding],
) -> proc_macro2::TokenStream {
    let skips: Vec<_> = caps.iter().filter(|b| is_skip_serde(&b.ty)).collect();
    if skips.is_empty() {
        return quote! {};
    }

    let skip_names: Vec<_> = skips.iter().map(|b| &b.name).collect();
    let cap_names: Vec<_> = caps.iter().map(|b| &b.name).collect();

    quote! {
        if false #(|| #skip_names.needs_rehydration())* {
            *self = Self::#var {
                #(#cap_names,)*
                __wait,
            };
            break 'step ::core::result::Result::Err(match self.rehydrate() {
                #rehyd_name::Ok => ::core::unreachable!("rehydration required"),
                needs => needs,
            });
        }
    }
}

fn make_rehydration(
    vis: &syn::Visibility,
    rehyd_name: &Ident,
    awaits: &[AwaitPoint],
    captures_at_await: &[Vec<Binding>],
    join_caps_at: &[Vec<Binding>],
    all_skips: &[Binding],
    output_ty: &Type,
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
) {
    let step_ret = quote! {
        ::core::result::Result<::core::task::Poll<#output_ty>, #rehyd_name>
    };

    if all_skips.is_empty() {
        let rehyd_enum = quote! {
            #vis enum #rehyd_name {
                Ok,
            }
        };
        let rehyd_method = quote! {
            pub fn rehydrate(&mut self) -> #rehyd_name {
                #rehyd_name::Ok
            }
        };
        return (rehyd_enum, rehyd_method, step_ret);
    }

    let skip_variants = all_skips.iter().map(|b| {
        let n = &b.name;
        let t = &b.ty;
        let var = needs_rehydration_variant(n);
        quote! {
            #var {
                #n: &'a mut #t,
            }
        }
    });

    let rehyd_enum = quote! {
        #vis enum #rehyd_name<'a> {
            Ok,
            #(#skip_variants,)*
        }
    };

    let mut arms = Vec::new();
    arms.push(quote! {
        Self::NotStarted | Self::Finished => #rehyd_name::Ok
    });

    for (i, (ap, caps)) in awaits.iter().zip(captures_at_await.iter()).enumerate() {
        let has_waiting_var =
            !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. });
        if has_waiting_var {
            let var = waiting_variant(&ap.name);
            let skip_in_var: Vec<_> = caps.iter().filter(|b| is_skip_serde(&b.ty)).collect();

            if skip_in_var.is_empty() {
                arms.push(quote! {
                    Self::#var { .. } => #rehyd_name::Ok
                });
            } else {
                let skip_pats: Vec<_> = skip_in_var.iter().map(|b| &b.name).collect();
                let checks = skip_in_var.iter().map(|b| {
                    let n = &b.name;
                    let needs_var = needs_rehydration_variant(n);
                    quote! {
                        if #n.needs_rehydration() {
                            return #rehyd_name::#needs_var { #n };
                        }
                    }
                });

                arms.push(quote! {
                    Self::#var { #(#skip_pats,)* .. } => {
                        #(#checks)*
                        #rehyd_name::Ok
                    }
                });
            }
        }

        if for_has_iter_await(&ap.kind) {
            let iter_var = waiting_iter_variant(i);
            arms.push(quote! {
                Self::#iter_var { .. } => #rehyd_name::Ok
            });
        }

        if needs_join(&ap.kind) {
            let after_var = join_variant(&ap.kind, i);
            let join_caps = effective_join_caps(ap, &join_caps_at[i]);
            let skip_in_join: Vec<_> = join_caps.iter().filter(|b| is_skip_serde(&b.ty)).collect();
            if skip_in_join.is_empty() {
                arms.push(quote! {
                    Self::#after_var { .. } => #rehyd_name::Ok
                });
            } else {
                let skip_pats: Vec<_> = skip_in_join.iter().map(|b| &b.name).collect();
                let checks = skip_in_join.iter().map(|b| {
                    let n = &b.name;
                    let needs_var = needs_rehydration_variant(n);
                    quote! {
                        if #n.needs_rehydration() {
                            return #rehyd_name::#needs_var { #n };
                        }
                    }
                });
                arms.push(quote! {
                    Self::#after_var { #(#skip_pats,)* .. } => {
                        #(#checks)*
                        #rehyd_name::Ok
                    }
                });
            }
        }

        if is_loop_kind(&ap.kind) {
            let head_var = loop_head_variant(i);
            arms.push(quote! {
                Self::#head_var { .. } => #rehyd_name::Ok
            });
        }
    }

    let rehyd_method = quote! {
        pub fn rehydrate(&mut self) -> #rehyd_name<'_> {
            match self {
                #(#arms,)*
            }
        }
    };

    let step_ret = quote! {
        ::core::result::Result<::core::task::Poll<#output_ty>, #rehyd_name<'_>>
    };

    (rehyd_enum, rehyd_method, step_ret)
}

fn make_getters(
    awaits: &[AwaitPoint],
    captures_at_await: &[Vec<Binding>],
    join_caps_at: &[Vec<Binding>],
) -> proc_macro2::TokenStream {
    let mut all = collect_all_bindings(captures_at_await);
    for (i, caps) in join_caps_at.iter().enumerate() {
        let caps = if let Some(ap) = awaits.get(i) {
            effective_join_caps(ap, caps)
        } else {
            caps.clone()
        };
        for b in caps {
            upsert_binding(&mut all, b);
        }
    }

    let getters = all.iter().filter(|b| !b.name.to_string().starts_with("__")).map(|b| {
        let name = &b.name;
        let ty = &b.ty;
        let getter = format_ident!("get_{}", name);

        let mut arms = Vec::new();
        for (i, (ap, caps)) in awaits.iter().zip(captures_at_await.iter()).enumerate() {
            let has_waiting_var =
                !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. });
            if has_waiting_var && caps.iter().any(|c| c.name == *name) {
                let var = waiting_variant(&ap.name);
                arms.push(quote! {
                    Self::#var { #name, .. } => ::core::option::Option::Some(#name),
                });
            }
            if for_has_iter_await(&ap.kind)
                && join_caps_at[i].iter().any(|c| c.name == *name)
            {
                let iter_var = waiting_iter_variant(i);
                arms.push(quote! {
                    Self::#iter_var { #name, .. } => ::core::option::Option::Some(#name),
                });
            }
            let join_caps = effective_join_caps(ap, &join_caps_at[i]);
            if needs_join(&ap.kind) && join_caps.iter().any(|c| c.name == *name) {
                let after_var = join_variant(&ap.kind, i);
                arms.push(quote! {
                    Self::#after_var { #name, .. } => ::core::option::Option::Some(#name),
                });
            }
            if is_loop_kind(&ap.kind) && join_caps_at[i].iter().any(|c| c.name == *name) {
                let head_var = loop_head_variant(i);
                arms.push(quote! {
                    Self::#head_var { #name, .. } => ::core::option::Option::Some(#name),
                });
            }
        }

        quote! {
            pub fn #getter(&self) -> ::core::option::Option<&#ty> {
                match self {
                    #(#arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    });

    quote! {
        #(#getters)*
    }
}
