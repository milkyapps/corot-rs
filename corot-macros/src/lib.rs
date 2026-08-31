use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Expr, ExprPath, Ident, ItemFn, Local, LocalInit, Pat, PatIdent, PatType,
    Path, Stmt, Type,
};

/// Suspension points: `let name: Type = <expr with await>`, or an `if` with a
/// single await in the condition, the `then` branch, or the `else` branch.
///
/// Await-in-condition must be a bare `expr.await` whose output type is `bool`.
/// Await inside then/else must be a typed `let name: Type = … .await`.
///
/// Locals that live across an await must be type-annotated.
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

        if is_loop_kind(&ap.kind) {
            let head_var = loop_head_variant(i);
            let head_fields = join_caps_at[i].iter().map(|b| field_tokens(b));
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

    let settle_arms = awaits.iter().map(|ap| {
        let var = waiting_variant(&ap.name);
        let ty = &ap.wait_ty;
        quote! {
            Self::#var { __wait, .. } => {
                let value = value
                    .downcast_ref::<#ty>()
                    .unwrap_or_else(|| panic!("settle_wait: expected {}", ::core::any::type_name::<#ty>()));
                *__wait = ::core::option::Option::Some(*value);
            }
        }
    });

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

        let cap_pats: Vec<_> = caps.iter().map(cap_pat).collect();

        if is_loop_kind(&ap.kind) {
            let head_var = loop_head_variant(i);
            let head_pats: Vec<_> = join_caps_at[i].iter().map(cap_pat).collect();
            let before_await = loop_before_await(ap);
            let go_wait = gen_go_waiting(&var, caps, &ap.base);
            step_arms.push(quote! {
                Self::#head_var { #(#head_pats,)* } => {
                    #(#before_await)*
                    #go_wait
                }
            });
        }

        let guard = rehydration_guard(&rehyd_name, &var, caps);
        let after_resume = gen_after_resume(i, ap, &join_caps_at[i]);
        let tail = match &ap.kind {
            SuspendKind::Loop { .. } => gen_goto_loop_head(i, &join_caps_at[i]),
            SuspendKind::IfThen { .. } | SuspendKind::IfElse { .. } => {
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
                "#[corot] unsupported await placement (supported: typed let; if with await \
                 in condition / then / else; loop with one await)",
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
    as_await_loop_stmt(stmt)
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

fn bare_await_base(cond: &Expr) -> Option<Expr> {
    match cond {
        Expr::Await(a) => Some(a.base.as_ref().clone()),
        Expr::Paren(p) => bare_await_base(&p.expr),
        Expr::Group(g) => bare_await_base(&g.expr),
        _ => None,
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
        | SuspendKind::Loop { after_await, .. } => after_await.iter().collect(),
        SuspendKind::IfCondition { .. } => Vec::new(),
    }
}

fn needs_join(kind: &SuspendKind) -> bool {
    matches!(
        kind,
        SuspendKind::IfThen { .. } | SuspendKind::IfElse { .. } | SuspendKind::Loop { .. }
    )
}

fn is_loop_kind(kind: &SuspendKind) -> bool {
    matches!(kind, SuspendKind::Loop { .. })
}

fn after_if_variant(index: usize) -> Ident {
    format_ident!("AfterIf{}", index)
}

fn after_loop_variant(index: usize) -> Ident {
    format_ident!("AfterLoop{}", index)
}

fn loop_head_variant(index: usize) -> Ident {
    format_ident!("LoopHead{}", index)
}

fn join_variant(kind: &SuspendKind, index: usize) -> Ident {
    if is_loop_kind(kind) {
        after_loop_variant(index)
    } else {
        after_if_variant(index)
    }
}

fn join_stmts_of(ap: &AwaitPoint) -> Option<&[Stmt]> {
    match &ap.kind {
        SuspendKind::IfThen { join_stmts, .. }
        | SuspendKind::IfElse { join_stmts, .. }
        | SuspendKind::Loop { join_stmts, .. } => Some(join_stmts.as_slice()),
        _ => None,
    }
}

fn loop_before_await(ap: &AwaitPoint) -> &[Stmt] {
    match &ap.kind {
        SuspendKind::Loop { before_await, .. } => before_await.as_slice(),
        _ => &[],
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
            | SuspendKind::Loop { join_stmts, .. } => {
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

fn gen_goto_loop_head(index: usize, join_caps: &[Binding]) -> proc_macro2::TokenStream {
    let var = loop_head_variant(index);
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
        SuspendKind::Plain { .. } | SuspendKind::IfCondition { .. } => quote! {
            #(#before)*
            #go_wait
        },
        SuspendKind::Loop { .. } => {
            let goto_head = gen_goto_loop_head(index, join_caps);
            quote! {
                #(#before)*
                #goto_head
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
        SuspendKind::IfThen { after_await, .. } | SuspendKind::IfElse { after_await, .. } => {
            quote! { #(#after_await)* }
        }
        SuspendKind::Loop { after_await, .. } => {
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
            contains_await(&e.expr) || e.arms.iter().any(|a| contains_await(&a.body))
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

    let getters = all.iter().map(|b| {
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
