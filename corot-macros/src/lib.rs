use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Expr, ExprPath, Ident, ItemFn, Local, LocalInit, Pat, PatIdent, PatType,
    Path, Stmt, Type,
};

/// Turns an `async fn` into a stepped coroutine enum.
///
/// Suspension points are statements that contain `.await` (typically
/// `let name: Type = <expr with await>;`). The `Type` is the settled wait
/// type (await output). Code before `.await` runs on the way into the wait;
/// code after `.await` in the same expression runs on resume.
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
    /// Statements before this await statement.
    before: Vec<Stmt>,
    /// Statement(s) to run on resume (await replaced by `tmp`).
    after_resume: Vec<Stmt>,
}

#[derive(Clone)]
struct Binding {
    name: Ident,
    ty: Type,
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

    let (awaits, after_last) = split_awaits(&input.block.stmts)?;

    let mut live: Vec<Binding> = Vec::new();
    let mut captures_at_await: Vec<Vec<Binding>> = Vec::new();

    for ap in &awaits {
        for stmt in &ap.before {
            if let Some(b) = typed_let_binding(stmt) {
                upsert_binding(&mut live, b);
            }
        }
        captures_at_await.push(live.clone());
        // Bindings introduced by the resume of this await become live afterward.
        for stmt in &ap.after_resume {
            if let Some(b) = typed_let_binding(stmt) {
                upsert_binding(&mut live, b);
            }
        }
    }

    let mut variants = vec![quote! { NotStarted }];
    for (ap, caps) in awaits.iter().zip(captures_at_await.iter()) {
        let var = waiting_variant(&ap.name);
        let cap_fields = caps.iter().map(|b| {
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
        });
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

    let mut step_arms = Vec::new();

    if awaits.is_empty() {
        step_arms.push(quote! {
            Self::NotStarted => {
                #(#after_last)*
                *self = Self::Finished;
                ::core::task::Poll::Ready(())
            }
        });
    } else {
        let first = &awaits[0];
        let before = &first.before;
        let var = waiting_variant(&first.name);
        let caps = &captures_at_await[0];
        let cap_moves = caps.iter().map(|b| {
            let n = &b.name;
            quote! { #n }
        });
        let base = &first.base;
        step_arms.push(quote! {
            Self::NotStarted => {
                #(#before)*
                let _ = #base;
                *self = Self::#var {
                    #(#cap_moves,)*
                    __wait: ::core::option::Option::None,
                };
                ::core::task::Poll::Pending
            }
        });
    }

    for i in 0..awaits.len() {
        let ap = &awaits[i];
        let var = waiting_variant(&ap.name);
        let caps = &captures_at_await[i];
        let tmp = &ap.tmp;
        let after_resume = &ap.after_resume;

        let cap_pats: Vec<_> = caps
            .iter()
            .map(|b| {
                let n = &b.name;
                quote! { #n }
            })
            .collect();

        let is_last = i + 1 == awaits.len();
        if is_last {
            step_arms.push(quote! {
                Self::#var { #(#cap_pats,)* __wait } => {
                    let #tmp = __wait.expect("call settle_wait before step");
                    #(#after_resume)*
                    #(#after_last)*
                    *self = Self::Finished;
                    ::core::task::Poll::Ready(())
                }
            });
        } else {
            let next = &awaits[i + 1];
            let next_before = &next.before;
            let next_var = waiting_variant(&next.name);
            let next_caps = &captures_at_await[i + 1];
            let next_cap_moves = next_caps.iter().map(|b| {
                let n = &b.name;
                quote! { #n }
            });
            let next_base = &next.base;
            step_arms.push(quote! {
                Self::#var { #(#cap_pats,)* __wait } => {
                    let #tmp = __wait.expect("call settle_wait before step");
                    #(#after_resume)*
                    #(#next_before)*
                    let _ = #next_base;
                    *self = Self::#next_var {
                        #(#next_cap_moves,)*
                        __wait: ::core::option::Option::None,
                    };
                    ::core::task::Poll::Pending
                }
            });
        }
    }

    step_arms.push(quote! {
        Self::Finished => ::core::task::Poll::Ready(())
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

    Ok(quote! {
        #serde_attrs
        #[allow(dead_code)]
        #vis enum #enum_name {
            #(#variants,)*
        }

        impl #enum_name {
            #settle_fn

            #[allow(unused_variables)]
            pub fn step(&mut self) -> ::core::task::Poll<()> {
                match ::core::mem::replace(self, Self::Finished) {
                    #(#step_arms,)*
                }
            }
        }

        #vis fn #fn_name() -> #enum_name {
            #enum_name::NotStarted
        }
    })
}

fn coroutine_name(fn_name: &Ident) -> Ident {
    let s = fn_name.to_string();
    let mut chars = s.chars();
    let pascal = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => s,
    };
    format_ident!("{}Coroutine", pascal)
}

fn waiting_variant(bind: &Ident) -> Ident {
    let s = bind.to_string();
    let mut chars = s.chars();
    let pascal = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => s,
    };
    format_ident!("Waiting{}", pascal)
}

fn split_awaits(stmts: &[Stmt]) -> syn::Result<(Vec<AwaitPoint>, Vec<Stmt>)> {
    let mut awaits = Vec::new();
    let mut current: Vec<Stmt> = Vec::new();

    for stmt in stmts {
        if let Some(ap) = as_await_stmt(stmt)? {
            let mut ap = ap;
            ap.before = std::mem::take(&mut current);
            awaits.push(ap);
        } else {
            current.push(stmt.clone());
        }
    }

    Ok((awaits, current))
}

fn as_await_stmt(stmt: &Stmt) -> syn::Result<Option<AwaitPoint>> {
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

    Ok(Some(AwaitPoint {
        name,
        tmp,
        wait_ty,
        base,
        before: Vec::new(),
        after_resume,
    }))
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
    let name = match pat_ident(pat) {
        Ok(n) => n,
        Err(_) => return None,
    };
    Some(Binding {
        name,
        ty: ty.as_ref().clone(),
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
    } else {
        live.push(b);
    }
}

/// True if the type path's last segment is `SkipSerde` (e.g. `SkipSerde<T>`,
/// `corot_rs::SkipSerde<T>`). Proc macros cannot check trait impls; this marker
/// is the opt-out for serde.
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
