//! Recognises the `Final` and `TypeAlias` forms of an annotation. A string annotation is judged by
//! the expression its content parses to.

use std::borrow::Cow;

use ruff_python_ast::Expr;
use ruff_python_parser::parse_expression;

/// How an annotation spells `Final`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalForm {
    /// Any annotation that is not `Final`, including a string that does not parse.
    NotFinal,
    /// `Final` with no type argument, such as `Final` or `typing.Final`.
    Bare,
    /// `Final[T]`, such as `Final[int]` or `typing.Final[int]`.
    Typed,
}

/// Classifies an annotation by its `Final` form.
pub fn final_form(annotation: &Expr) -> FinalForm {
    annotation_expr(annotation).map_or(FinalForm::NotFinal, |expr| classify_final(&expr))
}

/// Whether the annotation is `TypeAlias` or an attribute ending in `.TypeAlias`, bare or quoted.
pub fn is_type_alias(annotation: &Expr) -> bool {
    annotation_expr(annotation).is_some_and(|expr| ends_with_name(&expr, "TypeAlias"))
}

fn classify_final(expr: &Expr) -> FinalForm {
    match expr {
        Expr::Subscript(subscript) if ends_with_name(&subscript.value, "Final") => FinalForm::Typed,
        _ if ends_with_name(expr, "Final") => FinalForm::Bare,
        _ => FinalForm::NotFinal,
    }
}

/// The expression an annotation stands for. A string annotation stands for the expression its
/// content parses to, and nothing at all when the content does not parse.
fn annotation_expr(annotation: &Expr) -> Option<Cow<'_, Expr>> {
    match annotation {
        Expr::StringLiteral(literal) => {
            let parsed = parse_expression(literal.value.to_str()).ok()?;
            Some(Cow::Owned(*parsed.into_syntax().body))
        }
        _ => Some(Cow::Borrowed(annotation)),
    }
}

/// The identifier of a plain name, such as `x` in `x: int`, and `None` for any other expression.
pub fn plain_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(name) => Some(name.id.as_str()),
        _ => None,
    }
}

/// Whether `expr` is the name `name`, or an attribute whose last segment is `name`. This is the one
/// rule for recognising a well-known name, whether it is spelled bare or through a module.
pub fn ends_with_name(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Name(variable) => variable.id.as_str() == name,
        Expr::Attribute(attribute) => attribute.attr.as_str() == name,
        _ => false,
    }
}
