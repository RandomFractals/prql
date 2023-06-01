use anyhow::Result;
use itertools::Itertools;

use crate::ast::pl::*;
use crate::error::{Error, Reason, WithErrorInfo};

use super::Context;

/// Takes a resolved [Expr] and evaluates it a type expression that can be used to construct a type.
pub fn coerce_to_type(expr: Expr, context: &Context) -> Result<Ty, Error> {
    coerce_kind_to_set(expr.kind, context)
}

fn coerce_to_aliased_type(expr: Expr, context: &Context) -> Result<(Option<String>, Ty), Error> {
    let name = expr.alias;
    let expr = coerce_kind_to_set(expr.kind, context).map_err(|e| e.with_span(expr.span))?;

    Ok((name, expr))
}

fn coerce_kind_to_set(expr: ExprKind, context: &Context) -> Result<Ty, Error> {
    // already resolved type expressions (mostly primitives)
    if let ExprKind::Type(set_expr) = expr {
        return Ok(set_expr);
    }

    // singletons
    if let ExprKind::Literal(lit) = expr {
        return Ok(Ty {
            name: None,
            kind: TyKind::Singleton(lit),
        });
    }

    // tuples
    if let ExprKind::Tuple(mut elements) = expr {
        let mut set_elements = Vec::with_capacity(elements.len());

        // special case: {x..}
        if elements.len() == 1 {
            let only = elements.remove(0);
            if let ExprKind::Range(Range { start, end: None }) = only.kind {
                let inner = match start {
                    Some(x) => Some(coerce_to_type(*x, context)?),
                    None => None,
                };

                set_elements.push(TupleField::Wildcard(inner))
            } else {
                elements.push(only);
            }
        }

        for e in elements {
            let (name, ty) = coerce_to_aliased_type(e, context)?;
            let ty = Some(ty);

            set_elements.push(TupleField::Single(name, ty));
        }

        return Ok(Ty {
            name: None,
            kind: TyKind::Tuple(set_elements),
        });
    }

    // arrays
    if let ExprKind::Array(elements) = expr {
        if elements.len() != 1 {
            return Err(Error::new_simple(
                "For type expressions, arrays must contain exactly one element.",
            ));
        }
        let items_type = elements.into_iter().next().unwrap();
        let (_, items_type) = coerce_to_aliased_type(items_type, context)?;

        return Ok(Ty {
            name: None,
            kind: TyKind::Array(Box::new(items_type.kind)),
        });
    }

    // unions
    if let ExprKind::Binary {
        left,
        op: BinOp::Or,
        right,
    } = expr
    {
        let left = coerce_to_type(*left, context)?;
        let right = coerce_to_type(*right, context)?;

        // flatten nested unions
        let mut options = Vec::with_capacity(2);
        if let TyKind::Union(parts) = left.kind {
            options.extend(parts);
        } else {
            options.push((left.name.clone(), left));
        }
        if let TyKind::Union(parts) = right.kind {
            options.extend(parts);
        } else {
            options.push((right.name.clone(), right));
        }

        return Ok(Ty {
            name: None,
            kind: TyKind::Union(options),
        });
    }

    Err(Error::new_simple(format!(
        "not a type expression: {}",
        Expr::from(expr)
    )))
}

pub fn infer_type(node: &Expr) -> Result<Option<Ty>> {
    if let Some(ty) = &node.ty {
        return Ok(Some(ty.clone()));
    }

    let kind = match &node.kind {
        ExprKind::Literal(ref literal) => match literal {
            Literal::Null => TyKind::Singleton(Literal::Null),
            Literal::Integer(_) => TyKind::Primitive(PrimitiveSet::Int),
            Literal::Float(_) => TyKind::Primitive(PrimitiveSet::Float),
            Literal::Boolean(_) => TyKind::Primitive(PrimitiveSet::Bool),
            Literal::String(_) => TyKind::Primitive(PrimitiveSet::Text),
            Literal::Date(_) => TyKind::Primitive(PrimitiveSet::Date),
            Literal::Time(_) => TyKind::Primitive(PrimitiveSet::Time),
            Literal::Timestamp(_) => TyKind::Primitive(PrimitiveSet::Timestamp),
            Literal::ValueAndUnit(_) => return Ok(None), // TODO
        },

        ExprKind::Ident(_) | ExprKind::Pipeline(_) | ExprKind::FuncCall(_) => return Ok(None),

        ExprKind::SString(_) => return Ok(None),
        ExprKind::FString(_) => TyKind::Primitive(PrimitiveSet::Text),
        ExprKind::Range(_) => return Ok(None), // TODO

        ExprKind::TransformCall(_) => return Ok(None), // TODO
        ExprKind::Tuple(fields) => TyKind::Tuple(
            fields
                .iter()
                .map(|x| -> Result<_> {
                    let ty = infer_type(x)?;

                    Ok(TupleField::Single(None, ty))
                })
                .try_collect()?,
        ),

        _ => return Ok(None),
    };
    Ok(Some(Ty { kind, name: None }))
}

#[allow(dead_code)]
fn too_many_arguments(call: &FuncCall, expected_len: usize, passed_len: usize) -> Error {
    let err = Error::new(Reason::Expected {
        who: Some(format!("{}", call.name)),
        expected: format!("{} arguments", expected_len),
        found: format!("{}", passed_len),
    });
    if passed_len >= 2 {
        err.with_help(format!(
            "If you are calling a function, you may want to add parentheses `{} [{:?} {:?}]`",
            call.name, call.args[0], call.args[1]
        ))
    } else {
        err
    }
}

impl Context {
    /// Validates that found node has expected type. Returns assumed type of the node.
    pub fn validate_type<F>(
        &mut self,
        found: &mut Expr,
        expected: Option<&Ty>,
        who: &F,
    ) -> Result<(), Error>
    where
        F: Fn() -> Option<String>,
    {
        let found_ty = found.ty.clone();

        // infer
        let Some(expected) = expected else {
            // expected is none: there is no validation to be done
            return Ok(());
        };

        let Some(found_ty) = found_ty else {
            // found is none: infer from expected

            if found.lineage.is_none() && expected.is_relation() {
                // special case: infer a table type
                // inferred tables are needed for s-strings that represent tables
                // similarly as normal table references, we want to be able to infer columns
                // of this table, which means it needs to be defined somewhere in the module structure.
                let frame =
                    self.declare_table_for_literal(found.id.unwrap(), None, found.alias.clone());

                // override the empty frame with frame of the new table
                found.lineage = Some(frame)
            }

            // base case: infer expected type
            found.ty = Some(expected.clone());

            return Ok(());
        };

        let expected_is_above = match &mut found.kind {
            // special case of container type: tuple
            ExprKind::Tuple(found_fields) => {
                let ok = self.validate_tuple_type(found_fields, expected, who)?;
                if ok {
                    return Ok(());
                }
                false
            }

            // base case: compare types
            _ => expected.is_super_type_of(&found_ty),
        };
        if !expected_is_above {
            fn display_ty(ty: &Ty) -> String {
                if ty.is_tuple() {
                    "a tuple".to_string()
                } else {
                    format!("type `{}`", ty)
                }
            }

            let who = who();
            let is_join = who
                .as_ref()
                .map(|x| x.contains("std.join"))
                .unwrap_or_default();

            let e = Err(Error::new(Reason::Expected {
                who,
                expected: display_ty(expected),
                found: display_ty(&found_ty),
            })
            .with_span(found.span));

            if found_ty.is_function() && !expected.is_function() {
                let func_name = found.kind.as_func().and_then(|c| c.name_hint.as_ref());
                let to_what = func_name
                    .map(|n| format!("to function {n}"))
                    .unwrap_or_else(|| "in this function call?".to_string());

                return e.with_help(format!("Have you forgotten an argument {to_what}?"));
            };

            if is_join && found_ty.is_tuple() && !expected.is_tuple() {
                return e.with_help("Try using `(...)` instead of `{...}`");
            }

            return e;
        }
        Ok(())
    }

    fn validate_tuple_type<F>(
        &mut self,
        found_fields: &mut [Expr],
        expected: &Ty,
        who: &F,
    ) -> Result<bool, Error>
    where
        F: Fn() -> Option<String>,
    {
        let Some(expected_fields) = find_potential_tuple_fields(expected) else{
            return Ok(false);
        };

        let mut found = found_fields.iter_mut();

        for expected_field in expected_fields {
            match expected_field {
                TupleField::Single(_, expected_kind) => match found.next() {
                    Some(found_field) => {
                        self.validate_type(found_field, expected_kind.as_ref(), who)?
                    }
                    None => {
                        return Ok(false);
                    }
                },
                TupleField::Wildcard(expected_wildcard) => {
                    for found_field in found {
                        self.validate_type(found_field, expected_wildcard.as_ref(), who)?;
                    }
                    return Ok(true);
                }
            }
        }

        Ok(found.next().is_none())
    }
}

fn find_potential_tuple_fields(expected: &Ty) -> Option<&Vec<TupleField>> {
    match &expected.kind {
        TyKind::Tuple(fields) => Some(fields),
        TyKind::Union(variants) => {
            for (_, variant) in variants {
                if let Some(fields) = find_potential_tuple_fields(variant) {
                    return Some(fields);
                }
            }
            None
        }
        _ => None,
    }
}
