//! An example of using the `moniker` library to implement the simply typed
//! lambda calculus with records, variants, literals, and pattern matching.
//!
//! We use [bidirectional type checking](http://www.davidchristiansen.dk/tutorials/bidirectional.pdf)
//! to get some level of type inference.
//!
//! To implement pattern matching we referred to:
//!
//! - [The Locally Nameless Representation (Section 7.3)](https://www.chargueraud.org/research/2009/ln/main.pdf)
//! - [Towards a practical programming language based on dependent type theory (Chapter 2)]()

extern crate im;
#[macro_use]
extern crate moniker;

use im::HashMap;
use moniker::{Binder, BoundTerm, Embed, FreeVar, Scope, Var};
use std::rc::Rc;

/// Types
#[derive(Debug, Clone, BoundTerm)]
pub enum Type {
    /// Integers
    Int,
    /// Floating point numbers
    Float,
    /// Strings
    String,
    /// Function types
    Arrow(RcType, RcType),
    /// Record types
    Record(Vec<(String, RcType)>),
    /// Variant types
    Variant(Vec<(String, RcType)>),
}

/// Reference counted types
#[derive(Debug, Clone, BoundTerm)]
pub struct RcType {
    pub inner: Rc<Type>,
}

impl From<Type> for RcType {
    fn from(src: Type) -> RcType {
        RcType {
            inner: Rc::new(src),
        }
    }
}

/// Literal values
#[derive(Debug, Clone, PartialEq, BoundTerm, BoundPattern)]
pub enum Literal {
    /// Integer literals
    Int(i32),
    /// Floating point literals
    Float(f32),
    /// String literals
    String(String),
}

/// Patterns
#[derive(Debug, Clone, BoundPattern)]
pub enum Pattern {
    /// Patterns annotated with types
    Ann(RcPattern, Embed<RcType>),
    /// Literal patterns
    Literal(Literal),
    /// Patterns that bind variables
    Binder(Binder<String>),
    /// Record patterns
    Record(Vec<(String, RcPattern)>),
    /// Tag pattern
    Tag(String, RcPattern),
}

/// Reference counted patterns
#[derive(Debug, Clone, BoundPattern)]
pub struct RcPattern {
    pub inner: Rc<Pattern>,
}

impl From<Pattern> for RcPattern {
    fn from(src: Pattern) -> RcPattern {
        RcPattern {
            inner: Rc::new(src),
        }
    }
}

/// Expressions
#[derive(Debug, Clone, BoundTerm)]
pub enum Expr {
    /// Annotated expressions
    Ann(RcExpr, RcType),
    /// Literals
    Literal(Literal),
    /// Variables
    Var(Var<String>),
    /// Lambda expressions
    Lam(Scope<RcPattern, RcExpr>),
    /// Function application
    App(RcExpr, RcExpr),
    /// Record values
    Record(Vec<(String, RcExpr)>),
    /// Field projection on records
    Proj(RcExpr, String),
    /// Variant introduction
    Tag(String, RcExpr),
    /// Case expressions
    Case(RcExpr, Vec<Scope<RcPattern, RcExpr>>),
}

/// Reference counted expressions
#[derive(Debug, Clone, BoundTerm)]
pub struct RcExpr {
    pub inner: Rc<Expr>,
}

impl From<Expr> for RcExpr {
    fn from(src: Expr) -> RcExpr {
        RcExpr {
            inner: Rc::new(src),
        }
    }
}

impl RcExpr {
    // FIXME: auto-derive this somehow!
    fn substs<N>(&self, mappings: &[(N, RcExpr)]) -> RcExpr
    where
        Var<String>: PartialEq<N>,
    {
        match *self.inner {
            Expr::Ann(ref expr, ref ty) => {
                RcExpr::from(Expr::Ann(expr.substs(mappings), ty.clone()))
            },
            Expr::Var(ref var) => match mappings.iter().find(|&(name, _)| var == name) {
                Some((_, ref replacement)) => replacement.clone(),
                None => self.clone(),
            },
            Expr::Literal(_) => self.clone(),
            Expr::Lam(ref scope) => RcExpr::from(Expr::Lam(Scope {
                unsafe_pattern: scope.unsafe_pattern.clone(),
                unsafe_body: scope.unsafe_body.substs(mappings),
            })),
            Expr::App(ref fun, ref arg) => {
                RcExpr::from(Expr::App(fun.substs(mappings), arg.substs(mappings)))
            },
            Expr::Record(ref fields) => {
                let fields = fields
                    .iter()
                    .map(|&(ref label, ref elem)| (label.clone(), elem.substs(mappings)))
                    .collect();

                RcExpr::from(Expr::Record(fields))
            },
            Expr::Proj(ref expr, ref label) => {
                RcExpr::from(Expr::Proj(expr.substs(mappings), label.clone()))
            },
            Expr::Tag(ref label, ref expr) => {
                RcExpr::from(Expr::Tag(label.clone(), expr.substs(mappings)))
            },
            Expr::Case(ref expr, ref clauses) => RcExpr::from(Expr::Case(
                expr.substs(mappings),
                clauses
                    .iter()
                    .map(|scope| Scope {
                        unsafe_pattern: scope.unsafe_pattern.clone(), // subst?
                        unsafe_body: scope.unsafe_body.substs(mappings),
                    })
                    .collect(),
            )),
        }
    }
}

/// Evaluate an expression into its normal form
pub fn eval(expr: &RcExpr) -> RcExpr {
    match *expr.inner {
        Expr::Ann(ref expr, _) => eval(expr),
        Expr::Literal(_) | Expr::Var(_) | Expr::Lam(_) => expr.clone(),
        Expr::App(ref fun, ref arg) => match *eval(fun).inner {
            Expr::Lam(ref scope) => {
                let (pattern, body) = scope.clone().unbind();
                match match_expr(&pattern, &eval(arg)) {
                    None => expr.clone(), // stuck
                    Some(mappings) => eval(&body.substs(&mappings)),
                }
            },
            _ => expr.clone(),
        },
        Expr::Record(ref fields) => {
            let fields = fields
                .iter()
                .map(|&(ref label, ref elem)| (label.clone(), eval(elem)))
                .collect();

            RcExpr::from(Expr::Record(fields))
        },
        Expr::Proj(ref expr, ref label) => {
            let expr = eval(expr);

            if let Expr::Record(ref fields) = *expr.inner {
                if let Some(&(_, ref e)) = fields.iter().find(|&(ref l, _)| l == label) {
                    return e.clone();
                }
            }

            expr
        },
        Expr::Tag(ref label, ref expr) => RcExpr::from(Expr::Tag(label.clone(), eval(expr))),
        Expr::Case(ref arg, ref clauses) => {
            for clause in clauses {
                let (pattern, body) = clause.clone().unbind();
                match match_expr(&pattern, &eval(arg)) {
                    None => {}, // stuck
                    Some(mappings) => return eval(&body.substs(&mappings)),
                }
            }
            expr.clone() // stuck
        },
    }
}

/// If the pattern matches the expression, this function returns the
/// substitutions needed to apply the pattern to some body expression
///
/// We assume that the given expression has been evaluated first!
pub fn match_expr(pattern: &RcPattern, expr: &RcExpr) -> Option<Vec<(FreeVar<String>, RcExpr)>> {
    match (&*pattern.inner, &*expr.inner) {
        (&Pattern::Ann(ref pattern, _), _) => match_expr(pattern, expr),
        (&Pattern::Literal(ref pattern_lit), &Expr::Literal(ref expr_lit))
            if pattern_lit == expr_lit =>
        {
            Some(vec![])
        },
        (&Pattern::Binder(Binder(ref free_var)), _) => Some(vec![(free_var.clone(), expr.clone())]),
        (&Pattern::Record(ref pattern_fields), &Expr::Record(ref expr_fields))
            if pattern_fields.len() == expr_fields.len() =>
        {
            // FIXME: allow out-of-order fields in records
            let mut mappings = Vec::new();
            for (pattern_field, expr_field) in <_>::zip(pattern_fields.iter(), expr_fields.iter()) {
                if pattern_field.0 != expr_field.0 {
                    return None;
                } else {
                    mappings.extend(match_expr(&pattern_field.1, &expr_field.1)?);
                }
            }
            Some(mappings)
        }
        (&Pattern::Tag(ref pattern_label, ref pattern), &Expr::Tag(ref expr_label, ref expr))
            if pattern_label == expr_label =>
        {
            match_expr(pattern, expr)
        },
        (_, _) => None,
    }
}

/// A context containing a series of type annotations
type Context = HashMap<FreeVar<String>, RcType>;

/// Check that a (potentially ambiguous) expression conforms to a given ype
pub fn check_expr(context: &Context, expr: &RcExpr, expected_ty: &RcType) -> Result<(), String> {
    match (&*expr.inner, &*expected_ty.inner) {
        (&Expr::Lam(ref scope), &Type::Arrow(ref param_ty, ref ret_ty)) => {
            let (pattern, body) = scope.clone().unbind();
            let bindings = check_pattern(context, &pattern, param_ty)?;
            return check_expr(&(context + &bindings), &body, ret_ty);
        },
        (&Expr::Tag(ref label, ref expr), &Type::Variant(ref variants)) => {
            return match variants.iter().find(|&(l, _)| l == label) {
                None => Err(format!(
                    "variant type did not contain the label `{}`",
                    label
                )),
                Some(&(_, ref ty)) => check_expr(context, expr, ty),
            };
        },
        (&Expr::Case(ref expr, ref clauses), _) => {
            let expr_ty = infer_expr(context, expr)?;
            for clause in clauses {
                let (pattern, body) = clause.clone().unbind();
                let bindings = check_pattern(context, &pattern, &expr_ty)?;
                check_expr(&(context + &bindings), &body, expected_ty)?;
            }
            return Ok(());
        },
        (_, _) => {},
    }

    let inferred_ty = infer_expr(&context, expr)?;

    // FIXME: allow out-of-order fields in records
    if RcType::term_eq(&inferred_ty, expected_ty) {
        Ok(())
    } else {
        Err(format!(
            "type mismatch - found `{:?}` but expected `{:?}`",
            inferred_ty, expected_ty
        ))
    }
}

/// Synthesize the types of unambiguous expressions
pub fn infer_expr(context: &Context, expr: &RcExpr) -> Result<RcType, String> {
    match *expr.inner {
        Expr::Ann(ref expr, ref ty) => {
            check_expr(context, expr, ty)?;
            Ok(ty.clone())
        },
        Expr::Literal(Literal::Int(_)) => Ok(RcType::from(Type::Int)),
        Expr::Literal(Literal::Float(_)) => Ok(RcType::from(Type::Float)),
        Expr::Literal(Literal::String(_)) => Ok(RcType::from(Type::String)),
        Expr::Var(Var::Free(ref free_var)) => match context.get(free_var) {
            Some(term) => Ok((*term).clone()),
            None => Err(format!("`{:?}` not found in `{:?}`", free_var, context)),
        },
        Expr::Var(Var::Bound(_, _, _)) => panic!("encountered a bound variable"),
        Expr::Lam(ref scope) => {
            let (pattern, body) = scope.clone().unbind();
            let (ann, bindings) = infer_pattern(context, &pattern)?;
            let body_ty = infer_expr(&(context + &bindings), &body)?;
            Ok(RcType::from(Type::Arrow(ann, body_ty)))
        },
        Expr::App(ref fun, ref arg) => match *infer_expr(context, fun)?.inner {
            Type::Arrow(ref param_ty, ref ret_ty) => {
                let arg_ty = infer_expr(context, arg)?;
                if RcType::term_eq(param_ty, &arg_ty) {
                    Ok(ret_ty.clone())
                } else {
                    Err(format!(
                        "argument type mismatch - found `{:?}` but expected `{:?}`",
                        arg_ty, param_ty,
                    ))
                }
            },
            _ => Err(format!("`{:?}` is not a function", fun)),
        },
        Expr::Record(ref fields) => {
            let fields = fields
                .iter()
                .map(|&(ref label, ref expr)| Ok((label.clone(), infer_expr(context, expr)?)))
                .collect::<Result<_, String>>()?;

            Ok(RcType::from(Type::Record(fields)))
        },
        Expr::Proj(ref expr, ref label) => match *infer_expr(context, expr)?.inner {
            Type::Record(ref fields) => match fields.iter().find(|&(l, _)| l == label) {
                Some(&(_, ref ty)) => Ok(ty.clone()),
                None => Err(format!("field `{}` not found in type", label)),
            },
            _ => Err("record expected".to_string()),
        },
        Expr::Tag(_, _) => Err("type annotations needed".to_string()),
        Expr::Case(_, _) => Err("type annotations needed".to_string()),
    }
}

// TODO: Check pattern coverage/exhaustiveness (ie. if a series of patterns
// cover all cases)

/// Synthesize the types of unambiguous patterns
///
/// This function also returns a telescope that can be used to extend the typing
/// context with additional bindings that the pattern introduces.
pub fn check_pattern(
    context: &Context,
    pattern: &RcPattern,
    expected_ty: &RcType,
) -> Result<Context, String> {
    match (&*pattern.inner, &*expected_ty.inner) {
        (&Pattern::Binder(Binder(ref free_var)), _) => {
            return Ok(Context::new().insert(free_var.clone(), expected_ty.clone()));
        },
        (&Pattern::Tag(ref label, ref pattern), &Type::Variant(ref variants)) => {
            return match variants.iter().find(|&(l, _)| l == label) {
                None => Err(format!(
                    "variant type did not contain the label `{}`",
                    label
                )),
                Some(&(_, ref ty)) => check_pattern(context, pattern, ty),
            };
        },
        (_, _) => {},
    }

    let (inferred_ty, telescope) = infer_pattern(&context, pattern)?;

    // FIXME: allow out-of-order fields in records
    if RcType::term_eq(&inferred_ty, expected_ty) {
        Ok(telescope)
    } else {
        Err(format!(
            "type mismatch - found `{:?}` but expected `{:?}`",
            inferred_ty, expected_ty
        ))
    }
}

/// Check that a (potentially ambiguous) pattern conforms to a given type
///
/// This function also returns a telescope that can be used to extend the typing
/// context with additional bindings that the pattern introduces.
pub fn infer_pattern(context: &Context, expr: &RcPattern) -> Result<(RcType, Context), String> {
    match *expr.inner {
        Pattern::Ann(ref pattern, Embed(ref ty)) => {
            let telescope = check_pattern(context, pattern, ty)?;
            Ok((ty.clone(), telescope))
        },
        Pattern::Literal(Literal::Int(_)) => Ok((RcType::from(Type::Int), Context::new())),
        Pattern::Literal(Literal::Float(_)) => Ok((RcType::from(Type::Float), Context::new())),
        Pattern::Literal(Literal::String(_)) => Ok((RcType::from(Type::String), Context::new())),
        Pattern::Binder(_) => Err("type annotations needed".to_string()),
        Pattern::Record(ref fields) => {
            let mut telescope = Context::new();

            let fields = fields
                .iter()
                .map(|&(ref label, ref pattern)| {
                    let (pattern_ty, pattern_telescope) = infer_pattern(context, pattern)?;
                    telescope.extend(pattern_telescope);
                    Ok((label.clone(), pattern_ty))
                })
                .collect::<Result<_, String>>()?;

            Ok((RcType::from(Type::Record(fields)), telescope))
        },
        Pattern::Tag(_, _) => Err("type annotations needed".to_string()),
    }
}

#[test]
fn test_infer_expr() {
    // expr = (\x : Int -> x)
    let expr = RcExpr::from(Expr::Lam(Scope::new(
        RcPattern::from(Pattern::Ann(
            RcPattern::from(Pattern::Binder(Binder::user("x"))),
            Embed(RcType::from(Type::Int)),
        )),
        RcExpr::from(Expr::Var(Var::user("x"))),
    )));

    assert_term_eq!(
        infer_expr(&Context::new(), &expr).unwrap(),
        RcType::from(Type::Arrow(
            RcType::from(Type::Int),
            RcType::from(Type::Int)
        )),
    );
}

#[test]
fn test_infer_app_expr() {
    // expr = (\x -> x : Int -> Int) 1
    let expr = RcExpr::from(Expr::App(
        RcExpr::from(Expr::Ann(
            RcExpr::from(Expr::Lam(Scope::new(
                RcPattern::from(Pattern::Binder(Binder::user("x"))),
                RcExpr::from(Expr::Var(Var::user("x"))),
            ))),
            RcType::from(Type::Arrow(
                RcType::from(Type::Int),
                RcType::from(Type::Int),
            )),
        )),
        RcExpr::from(Expr::Literal(Literal::Int(1))),
    ));

    assert_term_eq!(
        infer_expr(&Context::new(), &expr).unwrap(),
        RcType::from(Type::Int),
    );
}

#[test]
fn test_infer_expr_record1() {
    // expr = \{ x = a : Int, y = b : String } -> b
    let expr = RcExpr::from(Expr::Lam(Scope::new(
        RcPattern::from(Pattern::Record(vec![
            (
                String::from("x"),
                RcPattern::from(Pattern::Ann(
                    RcPattern::from(Pattern::Binder(Binder::user("a"))),
                    Embed(RcType::from(Type::Int)),
                )),
            ),
            (
                String::from("y"),
                RcPattern::from(Pattern::Ann(
                    RcPattern::from(Pattern::Binder(Binder::user("b"))),
                    Embed(RcType::from(Type::String)),
                )),
            ),
        ])),
        RcExpr::from(Expr::Var(Var::user("b"))),
    )));

    assert_term_eq!(
        infer_expr(&Context::new(), &expr).unwrap(),
        RcType::from(Type::Arrow(
            RcType::from(Type::Record(vec![
                (String::from("x"), RcType::from(Type::Int)),
                (String::from("y"), RcType::from(Type::String)),
            ])),
            RcType::from(Type::String),
        )),
    );
}

#[test]
fn test_infer_expr_record2() {
    // expr = \{ x = a : Int, y = b : String, z = c : Float } -> { x = a, y = b, z = c }
    let expr = RcExpr::from(Expr::Lam(Scope::new(
        RcPattern::from(Pattern::Record(vec![
            (
                String::from("x"),
                RcPattern::from(Pattern::Ann(
                    RcPattern::from(Pattern::Binder(Binder::user("a"))),
                    Embed(RcType::from(Type::Int)),
                )),
            ),
            (
                String::from("y"),
                RcPattern::from(Pattern::Ann(
                    RcPattern::from(Pattern::Binder(Binder::user("b"))),
                    Embed(RcType::from(Type::String)),
                )),
            ),
            (
                String::from("z"),
                RcPattern::from(Pattern::Ann(
                    RcPattern::from(Pattern::Binder(Binder::user("c"))),
                    Embed(RcType::from(Type::Float)),
                )),
            ),
        ])),
        RcExpr::from(Expr::Record(vec![
            (String::from("x"), RcExpr::from(Expr::Var(Var::user("a")))),
            (String::from("y"), RcExpr::from(Expr::Var(Var::user("b")))),
            (String::from("z"), RcExpr::from(Expr::Var(Var::user("c")))),
        ])),
    )));

    assert_term_eq!(
        infer_expr(&Context::new(), &expr).unwrap(),
        RcType::from(Type::Arrow(
            RcType::from(Type::Record(vec![
                (String::from("x"), RcType::from(Type::Int)),
                (String::from("y"), RcType::from(Type::String)),
                (String::from("z"), RcType::from(Type::Float)),
            ])),
            RcType::from(Type::Record(vec![
                (String::from("x"), RcType::from(Type::Int)),
                (String::from("y"), RcType::from(Type::String)),
                (String::from("z"), RcType::from(Type::Float)),
            ])),
        )),
    );
}

// TODO: Use property testing for this!
// http://janmidtgaard.dk/papers/Midtgaard-al%3AICFP17-full.pdf

fn main() {}
