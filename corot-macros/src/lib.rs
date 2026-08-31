use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Expr, ExprPath, Ident, ItemFn, Lit, Local, LocalInit, Pat, PatIdent, PatType,
    Path, Stmt, Type,
};

/// Suspension points: typed `let` awaits; `if` / `loop` / `for` / `match` with a
/// single await in a supported position.
///
/// - `if`: condition (`expr.await` → bool), then, or else
/// - `match`: scrutinee (`expr.await`), one arm body, or one guard (`expr.await` → bool)
/// - `for`: range literal / `(range).await`, optional body await
///
/// Locals that live across an await must be type-annotated.
/// Match scrutinee types are inferred from arm pattern literals when possible.
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
    /// Type provided by `settle_wait` (await output).
    wait_ty: Type,
    /// Expression evaluated before suspending (the await receiver/base).
    base: Expr,
    /// Statements before this await's statement/`if`.
    before: Vec<Stmt>,
    kind: SuspendKind,
}

enum SuspendKind {
    /// Top-level `let name: Ty = <expr with await>;`
    Plain {
        after_resume: Vec<Stmt>,
    },
    /// `if EXPR.await { then } else { else }` (EXPR.await is the whole condition)
    IfCondition {
        resume_cond: Expr,
        then_branch: syn::Block,
        else_branch: Option<Box<Expr>>,
    },
    /// Await only inside the then branch.
    IfThen {
        cond: Expr,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        else_branch: Option<Box<Expr>>,
        /// Stmts after the `if` until the next await / end (run in `AfterIfN`).
        join_stmts: Vec<Stmt>,
    },
    /// Await only inside the else branch.
    IfElse {
        cond: Expr,
        then_branch: syn::Block,
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `loop { before; let name: Ty = ….await; after; }` with optional `break`.
    Loop {
        before_await: Vec<Stmt>,
        after_await: Vec<Stmt>,
        join_stmts: Vec<Stmt>,
    },
    /// `for x in start..end { … }` or `for x in (start..end).await { … }`
    /// (ranges only for now). Optional await on the iterable; optional await in the body.
    For {
        item: Ident,
        item_ty: Type,
        iter_ty: Type,
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
}

#[derive(Clone)]
struct Binding {
    name: Ident,
    ty: Type,
    mutable: bool,
}

struct PlainAwait {
    name: Ident,
    tmp: Ident,
    wait_ty: Type,
    base: Expr,
    after_resume: Vec<Stmt>,
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

    let (mut awaits, mut after_last) = split_awaits(&input.block.stmts)?;
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
            SuspendKind::IfThen { before_await, .. }
            | SuspendKind::IfElse { before_await, .. }
            | SuspendKind::Loop { before_await, .. } => {
                for stmt in before_await {
                    if let Some(b) = typed_let_binding(stmt) {
                        upsert_binding(&mut live, b);
                    }
                }
            }
            SuspendKind::For {
                item,
                item_ty,
                iter_ty,
                before_await,
                ..
            } => {
                upsert_binding(
                    &mut live,
                    Binding {
                        name: format_ident!("__iter"),
                        ty: iter_ty.clone(),
                        mutable: true,
                    },
                );
                upsert_binding(
                    &mut live,
                    Binding {
                        name: item.clone(),
                        ty: item_ty.clone(),
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
            _ => {}
        }
        captures_at_await.push(live.clone());
        for stmt in after_resume_stmts(ap) {
            if let Some(b) = typed_let_binding(stmt) {
                upsert_binding(&mut live, b);
            }
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
            let iter_ty = for_iter_ty(&ap.kind).expect("for iter await");
            let wait_skip = cfg!(feature = "serde") && is_skip_serde(iter_ty);
            let wait_field = if wait_skip {
                quote! {
                    #[serde(skip)]
                    __wait: ::core::option::Option<#iter_ty>,
                }
            } else {
                quote! {
                    __wait: ::core::option::Option<#iter_ty>,
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
            let join_fields = join_caps_at[i].iter().map(|b| field_tokens(b));
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
            let iter_ty = for_iter_ty(&ap.kind).unwrap();
            settle_arms.push(quote! {
                Self::#iter_var { __wait, .. } => {
                    let value = value
                        .downcast_ref::<#iter_ty>()
                        .unwrap_or_else(|| panic!("settle_wait: expected {}", ::core::any::type_name::<#iter_ty>()));
                    *__wait = ::core::option::Option::Some(::core::clone::Clone::clone(value));
                }
            });
        }
        if !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. }) {
            let var = waiting_variant(&ap.name);
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
        step_arms.push(quote! {
            Self::NotStarted => {
                #(#after_last)*
                *self = Self::Finished;
                break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
            }
        });
    } else {
        let enter = gen_enter_await(
            0,
            &awaits[0],
            &waiting_variant(&awaits[0].name),
            &captures_at_await[0],
            &join_caps_at[0],
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
                    ..
                } => {
                    let goto_after = gen_goto_join(ap, i, &join_caps_at[i]);
                    let some_body = if *has_body_await {
                        let go_wait = gen_go_waiting(&var, caps, &ap.base);
                        quote! {
                            #(#before_await)*
                            #go_wait
                        }
                    } else {
                        let goto_head = gen_goto_loop_head(i, ap, &join_caps_at[i]);
                        quote! {
                            #(#before_await)*
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
                SuspendKind::Loop { before_await, .. } => {
                    let go_wait = gen_go_waiting(&var, caps, &ap.base);
                    step_arms.push(quote! {
                        Self::#head_var { #(#head_pats,)* } => {
                            #(#before_await)*
                            #go_wait
                        }
                    });
                }
                _ => {}
            }
        }

        if !matches!(&ap.kind, SuspendKind::For { has_body_await: false, .. }) {
            let cap_pats: Vec<_> = caps.iter().map(cap_pat).collect();
            let guard = rehydration_guard(&rehyd_name, &var, caps);
            let after_resume = gen_after_resume(i, ap, &join_caps_at[i]);
            let tail = match &ap.kind {
                SuspendKind::Loop { .. } | SuspendKind::For { .. } => {
                    gen_goto_loop_head(i, ap, &join_caps_at[i])
                }
                SuspendKind::IfThen { .. }
                | SuspendKind::IfElse { .. }
                | SuspendKind::MatchArm { .. }
                | SuspendKind::MatchGuard { .. } => {
                    gen_goto_join(ap, i, &join_caps_at[i])
                }
                _ => gen_join_tail(i, &awaits, &captures_at_await, &join_caps_at, &after_last),
            };

            step_arms.push(quote! {
                Self::#var { #(#cap_pats,)* __wait } => {
                    #guard
                    let #tmp = __wait.expect("call settle_wait before step");
                    #after_resume
                    #tail
                }
            });
        }

        if needs_join(&ap.kind) {
            let after_var = join_variant(&ap.kind, i);
            let join_caps = &join_caps_at[i];
            let join_pats: Vec<_> = join_caps.iter().map(cap_pat).collect();
            let join_stmts = join_stmts_of(ap).unwrap_or(&[]);
            let after_join =
                gen_join_tail(i, &awaits, &captures_at_await, &join_caps_at, &after_last);
            step_arms.push(quote! {
                Self::#after_var { #(#join_pats,)* } => {
                    #(#join_stmts)*
                    #after_join
                }
            });
        }
    }

    step_arms.push(quote! {
        Self::Finished => {
            break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
        }
    });

    let settle_fn = if awaits.is_empty() {
        quote! {}
    } else {
        quote! {
            pub fn settle_wait(&mut self, value: &dyn ::std::any::Any) {
                match self {
                    #(#settle_arms)*
                    _ => panic!("settle_wait called when not waiting"),
                }
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

    let (rehyd_enum, rehyd_method, step_ret) =
        make_rehydration(&vis, &rehyd_name, &awaits, &captures_at_await, &join_caps_at, &all_skips);
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

fn split_awaits(stmts: &[Stmt]) -> syn::Result<(Vec<AwaitPoint>, Vec<Stmt>)> {
    let mut awaits = Vec::new();
    let mut current: Vec<Stmt> = Vec::new();

    for stmt in stmts {
        if let Some(ap) = as_await_stmt(stmt, awaits.len())? {
            let mut ap = ap;
            ap.before = std::mem::take(&mut current);
            awaits.push(ap);
        } else if stmt_contains_await(stmt) {
            return Err(syn::Error::new_spanned(
                stmt,
                "#[corot] unsupported await placement (supported: typed let; if; match; loop; for range)",
            ));
        } else {
            current.push(stmt.clone());
        }
    }

    Ok((awaits, current))
}

fn as_await_stmt(stmt: &Stmt, index: usize) -> syn::Result<Option<AwaitPoint>> {
    if let Some(plain) = as_plain_await_let(stmt)? {
        return Ok(Some(AwaitPoint {
            name: plain.name,
            tmp: plain.tmp,
            wait_ty: plain.wait_ty,
            base: plain.base,
            before: Vec::new(),
            kind: SuspendKind::Plain {
                after_resume: plain.after_resume,
            },
        }));
    }
    if let Some(ap) = as_await_if_stmt(stmt, index)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_loop_stmt(stmt)? {
        return Ok(Some(ap));
    }
    if let Some(ap) = as_await_for_stmt(stmt)? {
        return Ok(Some(ap));
    }
    as_await_match_stmt(stmt, index)
}

fn as_plain_await_let(stmt: &Stmt) -> syn::Result<Option<PlainAwait>> {
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

    if diverge.is_some() {
        return Err(syn::Error::new_spanned(
            stmt,
            "#[corot] does not support let ... else with await",
        ));
    }

    if !contains_await(expr) {
        return Ok(None);
    }

    let (name, wait_ty) = match pat {
        Pat::Type(PatType { pat, ty, .. }) => {
            let name = pat_ident(pat)?;
            (name, ty.as_ref().clone())
        }
        _ => {
            return Err(syn::Error::new_spanned(
                pat,
                "await bindings must be written as `let name: Type = <expr with await>` \
                 (Type is the settle/await-output type)",
            ));
        }
    };

    let tmp = format_ident!("__await_{}", name);
    let mut resume_expr = expr.as_ref().clone();
    let base = replace_first_await(&mut resume_expr, ident_expr(&tmp))?;

    let after_resume = vec![Stmt::Local(Local {
        attrs: attrs.clone(),
        let_token: *let_token,
        pat: pat.clone(),
        init: Some(LocalInit {
            eq_token: *eq_token,
            expr: Box::new(resume_expr),
            diverge: None,
        }),
        semi_token: *semi_token,
    })];

    Ok(Some(PlainAwait {
        name,
        tmp,
        wait_ty,
        base,
        after_resume,
    }))
}

fn as_await_if_stmt(stmt: &Stmt, index: usize) -> syn::Result<Option<AwaitPoint>> {
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
                kind: SuspendKind::IfCondition {
                    resume_cond: ident_expr(&tmp),
                    then_branch: expr_if.then_branch.clone(),
                    else_branch: expr_if.else_branch.as_ref().map(|(_, e)| e.clone()),
                },
            }))
        }
        (false, true, false) => {
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(&expr_if.then_branch.stmts)?;
            Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
                kind: SuspendKind::IfThen {
                    cond: expr_if.cond.as_ref().clone(),
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
            let else_stmts = else_block_stmts(else_expr)?;
            let (before_await, plain, after_await) =
                extract_single_await_from_stmts(else_stmts)?;
            Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
                kind: SuspendKind::IfElse {
                    cond: expr_if.cond.as_ref().clone(),
                    then_branch: expr_if.then_branch.clone(),
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
        _ => Err(syn::Error::new_spanned(
            stmt,
            "#[corot] supports at most one await in an if, and only in the condition, \
             or only in then, or only in else",
        )),
    }
}

fn as_await_loop_stmt(stmt: &Stmt) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::Loop(expr_loop), _) = stmt else {
        return Ok(None);
    };
    if expr_loop.label.is_some() {
        return Err(syn::Error::new_spanned(
            stmt,
            "#[corot] labeled loops are not supported yet",
        ));
    }
    if !expr_loop.body.stmts.iter().any(stmt_contains_await) {
        return Ok(None);
    }
    let (before_await, plain, after_await) =
        extract_single_await_from_stmts(&expr_loop.body.stmts)?;
    Ok(Some(AwaitPoint {
        name: plain.name,
        tmp: plain.tmp,
        wait_ty: plain.wait_ty,
        base: plain.base,
        before: Vec::new(),
        kind: SuspendKind::Loop {
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

fn as_await_for_stmt(stmt: &Stmt) -> syn::Result<Option<AwaitPoint>> {
    let Stmt::Expr(Expr::ForLoop(expr_for), _) = stmt else {
        return Ok(None);
    };

    let has_iter_await = contains_await(&expr_for.expr);
    let has_body_await = expr_for.body.stmts.iter().any(stmt_contains_await);
    if !has_iter_await && !has_body_await {
        return Ok(None);
    }

    let item = pat_ident(&expr_for.pat)?;

    let (iter_expr, iter_await_base, item_ty, iter_ty) = if let Some(base) =
        bare_await_base(&expr_for.expr)
    {
        let (item_ty, iter_ty) = range_types(&base)?;
        // Drop outer parens so `let _ = 0..3` isn't warned as unused_parens.
        (None, Some(unwrap_parens(base)), item_ty, iter_ty)
    } else {
        let (item_ty, iter_ty) = range_types(&expr_for.expr)?;
        (
            Some(expr_for.expr.as_ref().clone()),
            None,
            item_ty,
            iter_ty,
        )
    };

    let (before_await, plain, after_await) = if has_body_await {
        extract_single_await_from_stmts(&expr_for.body.stmts)?
    } else {
        (
            expr_for.body.stmts.clone(),
            PlainAwait {
                name: format_ident!("iter"),
                tmp: format_ident!("__await_iter"),
                wait_ty: iter_ty.clone(),
                base: iter_await_base
                    .clone()
                    .unwrap_or_else(|| syn::parse_quote!(())),
                after_resume: Vec::new(),
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
        kind: SuspendKind::For {
            item,
            item_ty,
            iter_ty,
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

fn as_await_match_stmt(stmt: &Stmt, index: usize) -> syn::Result<Option<AwaitPoint>> {
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
            let (before_await, plain, after_await) = extract_single_await_from_stmts(&stmts)?;
            let sus_guard = sus.guard.as_ref().map(|(_, g)| g.clone());
            Ok(Some(AwaitPoint {
                name: plain.name,
                tmp: plain.tmp,
                wait_ty: plain.wait_ty,
                base: plain.base,
                before: Vec::new(),
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

fn range_types(expr: &Expr) -> syn::Result<(Type, Type)> {
    let expr = match expr {
        Expr::Paren(p) => p.expr.as_ref(),
        Expr::Group(g) => g.expr.as_ref(),
        other => other,
    };
    let Expr::Range(range) = expr else {
        return Err(syn::Error::new_spanned(
            expr,
            "#[corot] for-await currently only supports range literals like `0..3` \
             or `(0..3).await`",
        ));
    };
    let ty = int_lit_type(range.start.as_deref())
        .or_else(|| int_lit_type(range.end.as_deref()))
        .unwrap_or_else(|| syn::parse_quote!(i32));
    Ok((ty.clone(), syn::parse_quote!(::std::ops::Range<#ty>)))
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
            "#[corot] only supports `else { ... }` (not else-if) when awaiting in else",
        )),
    }
}

fn extract_single_await_from_stmts(
    stmts: &[Stmt],
) -> syn::Result<(Vec<Stmt>, PlainAwait, Vec<Stmt>)> {
    let mut before = Vec::new();
    let mut found: Option<PlainAwait> = None;
    let mut after = Vec::new();

    for stmt in stmts {
        if found.is_none() {
            if let Some(plain) = as_plain_await_let(stmt)? {
                found = Some(plain);
            } else if stmt_contains_await(stmt) {
                return Err(syn::Error::new_spanned(
                    stmt,
                    "#[corot] await inside if branches must be a typed let binding",
                ));
            } else {
                before.push(stmt.clone());
            }
        } else if stmt_contains_await(stmt) {
            return Err(syn::Error::new_spanned(
                stmt,
                "#[corot] only one await is supported inside an if branch",
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
        | SuspendKind::For { after_await, .. }
        | SuspendKind::MatchArm { after_await, .. } => after_await.iter().collect(),
        SuspendKind::IfCondition { .. }
        | SuspendKind::MatchScrutinee { .. }
        | SuspendKind::MatchGuard { .. } => Vec::new(),
    }
}

fn needs_join(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::IfThen { .. }
            | SuspendKind::IfElse { .. }
            | SuspendKind::Loop { .. }
            | SuspendKind::For { .. }
            | SuspendKind::MatchArm { .. }
            | SuspendKind::MatchGuard { .. }
    )
}

fn is_loop_kind(kind: &SuspendKind) -> bool {
    matches!(kind, SuspendKind::Loop { .. } | SuspendKind::For { .. })
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

fn for_iter_ty(kind: &SuspendKind) -> Option<&Type> {
    match kind {
        SuspendKind::For { iter_ty, .. } => Some(iter_ty),
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
        | SuspendKind::For { join_stmts, .. }
        | SuspendKind::MatchArm { join_stmts, .. }
        | SuspendKind::MatchGuard { join_stmts, .. } => Some(join_stmts.as_slice()),
        _ => None,
    }
}

fn loop_head_caps(ap: &AwaitPoint, join_caps: &[Binding]) -> Vec<Binding> {
    match &ap.kind {
        SuspendKind::For { iter_ty, .. } => {
            let mut caps = join_caps.to_vec();
            caps.push(Binding {
                name: format_ident!("__iter"),
                ty: iter_ty.clone(),
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
            | SuspendKind::For { join_stmts, .. }
            | SuspendKind::MatchArm { join_stmts, .. }
            | SuspendKind::MatchGuard { join_stmts, .. } => {
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

fn else_expr_tokens(else_branch: &Option<Box<Expr>>) -> proc_macro2::TokenStream {
    match else_branch {
        Some(e) => quote! { #e },
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
) -> proc_macro2::TokenStream {
    let before = &ap.before;
    let go_wait = gen_go_waiting(waiting_var, caps, &ap.base);
    let non_suspend = gen_goto_join(ap, index, join_caps);

    match &ap.kind {
        SuspendKind::Plain { .. }
        | SuspendKind::IfCondition { .. }
        | SuspendKind::MatchScrutinee { .. } => quote! {
            #(#before)*
            #go_wait
        },
        SuspendKind::Loop { .. } => {
            let goto_head = gen_goto_loop_head(index, ap, join_caps);
            quote! {
                #(#before)*
                #goto_head
            }
        }
        SuspendKind::For {
            iter_ty,
            iter_expr,
            iter_await_base,
            ..
        } => {
            let goto_head = gen_goto_loop_head(index, ap, join_caps);
            if let Some(base) = iter_await_base {
                let iter_var = waiting_iter_variant(index);
                let join_moves = join_caps.iter().map(|b| {
                    let n = &b.name;
                    quote! { #n }
                });
                quote! {
                    #(#before)*
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
                    #(#before)*
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
            let else_body = else_expr_tokens(else_branch);
            quote! {
                #(#before)*
                if #cond {
                    #(#before_await)*
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
            before_await,
            ..
        } => {
            let then_stmts = &then_branch.stmts;
            quote! {
                #(#before)*
                if #cond {
                    #(#then_stmts)*
                    #non_suspend
                } else {
                    #(#before_await)*
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
                .map(|a| emit_sync_arm_with_join(a, &non_suspend));
            let after_arms = arms_after
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend));
            let guard = match sus_guard {
                Some(g) => quote! { if #g },
                None => quote! {},
            };
            quote! {
                #(#before)*
                match #scrutinee {
                    #(#before_arms)*
                    #sus_pat #guard => {
                        #(#before_await)*
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
                .map(|a| emit_sync_arm_with_join(a, &non_suspend));
            let after_arms = arms_after
                .iter()
                .map(|a| emit_sync_arm_with_join(a, &non_suspend));
            // Two-phase match so an irrefutable suspending pattern (e.g. `n`) does not
            // make later arms unreachable when the await-guard is stripped for enter.
            quote! {
                #(#before)*
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
    }
}

fn emit_sync_arm_with_join(
    arm: &syn::Arm,
    goto_join: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let attrs = &arm.attrs;
    let pat = &arm.pat;
    let guard = match &arm.guard {
        Some((_, g)) => quote! { if #g },
        None => quote! {},
    };
    match arm.body.as_ref() {
        Expr::Block(b) => {
            let stmts = &b.block.stmts;
            quote! {
                #(#attrs)*
                #pat #guard => {
                    #(#stmts)*
                    #goto_join
                }
            }
        }
        body => quote! {
            #(#attrs)*
            #pat #guard => {
                #body;
                #goto_join
            }
        },
    }
}

fn emit_arm_body(arm: &syn::Arm) -> proc_macro2::TokenStream {
    let attrs = &arm.attrs;
    let pat = &arm.pat;
    let guard = match &arm.guard {
        Some((_, g)) => quote! { if #g },
        None => quote! {},
    };
    let body = &arm.body;
    quote! {
        #(#attrs)*
        #pat #guard => #body,
    }
}

fn gen_after_resume(
    index: usize,
    ap: &AwaitPoint,
    join_caps: &[Binding],
) -> proc_macro2::TokenStream {
    match &ap.kind {
        SuspendKind::Plain { after_resume } => quote! { #(#after_resume)* },
        SuspendKind::IfCondition {
            resume_cond,
            then_branch,
            else_branch,
        } => {
            let then_stmts = &then_branch.stmts;
            match else_branch {
                Some(e) => quote! {
                    if #resume_cond {
                        #(#then_stmts)*
                    } else #e;
                },
                None => quote! {
                    if #resume_cond {
                        #(#then_stmts)*
                    }
                },
            }
        }
        SuspendKind::MatchScrutinee { arms } => {
            let tmp = &ap.tmp;
            let arm_tokens = arms.iter().map(emit_arm_body);
            quote! {
                match #tmp {
                    #(#arm_tokens)*
                }
            }
        }
        SuspendKind::IfThen { after_await, .. }
        | SuspendKind::IfElse { after_await, .. }
        | SuspendKind::MatchArm { after_await, .. } => {
            quote! { #(#after_await)* }
        }
        SuspendKind::MatchGuard {
            sus_pat,
            sus_body,
            arms_after,
            ..
        } => {
            let tmp = &ap.tmp;
            let else_branch = if arms_after.is_empty() {
                quote! {}
            } else {
                let after_tokens = arms_after.iter().map(emit_arm_body);
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
        SuspendKind::Loop { after_await, .. } | SuspendKind::For { after_await, .. } => {
            rewrite_loop_after_await(index, after_await, join_caps)
        }
    }
}

fn rewrite_loop_after_await(
    index: usize,
    stmts: &[Stmt],
    join_caps: &[Binding],
) -> proc_macro2::TokenStream {
    let parts = stmts.iter().map(|stmt| rewrite_loop_stmt(stmt, index, join_caps));
    quote! { #(#parts)* }
}

fn rewrite_loop_stmt(
    stmt: &Stmt,
    index: usize,
    join_caps: &[Binding],
) -> proc_macro2::TokenStream {
    match stmt {
        Stmt::Expr(Expr::Break(brk), _) if brk.label.is_none() && brk.expr.is_none() => {
            let var = after_loop_variant(index);
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
        Stmt::Expr(Expr::If(expr_if), _) => {
            let cond = &expr_if.cond;
            let then_parts = expr_if
                .then_branch
                .stmts
                .iter()
                .map(|s| rewrite_loop_stmt(s, index, join_caps));
            match &expr_if.else_branch {
                None => quote! {
                    if #cond {
                        #(#then_parts)*
                    }
                },
                Some((_, else_expr)) => match else_expr.as_ref() {
                    Expr::Block(b) => {
                        let else_parts = b
                            .block
                            .stmts
                            .iter()
                            .map(|s| rewrite_loop_stmt(s, index, join_caps));
                        quote! {
                            if #cond {
                                #(#then_parts)*
                            } else {
                                #(#else_parts)*
                            }
                        }
                    }
                    other => quote! {
                        if #cond {
                            #(#then_parts)*
                        } else #other;
                    },
                },
            }
        }
        other => quote! { #other },
    }
}

/// Continuation after await `completed_i` is fully done (including join).
fn gen_join_tail(
    completed_i: usize,
    awaits: &[AwaitPoint],
    captures_at_await: &[Vec<Binding>],
    join_caps_at: &[Vec<Binding>],
    after_last: &[Stmt],
) -> proc_macro2::TokenStream {
    if completed_i + 1 >= awaits.len() {
        quote! {
            #(#after_last)*
            *self = Self::Finished;
            break 'step ::core::result::Result::Ok(::core::task::Poll::Ready(()));
        }
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
        )
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
        }) => contains_await(&init.expr),
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
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
) {
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
        let step_ret = quote! {
            ::core::result::Result<::core::task::Poll<()>, #rehyd_name>
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

        if needs_join(&ap.kind) {
            let after_var = join_variant(&ap.kind, i);
            let join_caps = &join_caps_at[i];
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
        ::core::result::Result<::core::task::Poll<()>, #rehyd_name<'_>>
    };

    (rehyd_enum, rehyd_method, step_ret)
}

fn make_getters(
    awaits: &[AwaitPoint],
    captures_at_await: &[Vec<Binding>],
    join_caps_at: &[Vec<Binding>],
) -> proc_macro2::TokenStream {
    let mut all = collect_all_bindings(captures_at_await);
    for caps in join_caps_at {
        for b in caps {
            upsert_binding(&mut all, b.clone());
        }
    }

    let getters = all.iter().filter(|b| !b.name.to_string().starts_with("__")).map(|b| {
        let name = &b.name;
        let ty = &b.ty;
        let getter = format_ident!("get_{}", name);

        let mut arms = Vec::new();
        for (i, (ap, caps)) in awaits.iter().zip(captures_at_await.iter()).enumerate() {
            if caps.iter().any(|c| c.name == *name) {
                let var = waiting_variant(&ap.name);
                arms.push(quote! {
                    Self::#var { #name, .. } => ::core::option::Option::Some(#name),
                });
            }
            if needs_join(&ap.kind) && join_caps_at[i].iter().any(|c| c.name == *name) {
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
