//! The obvious values of obvious mode. A value is obvious when its type is clear from its text alone, so the
//! first binding of a name can go without an annotation: a literal, a non-empty container of literals of one
//! shape, or a call that constructs a class.

use std::path::Path;

use ruff_python_ast::{DictItem, Expr, ExprCall, ExprUnaryOp, Number, UnaryOp};

use super::annotations::plain_name;
use super::class_index::ClassIndex;

/// Standard library classes that are generic, so a call without arguments, such as `Counter()`, leaves the
/// type of their items unknown.
const KNOWN_GENERICS: [&str; 13] = [
    "Counter",
    "OrderedDict",
    "ChainMap",
    "UserDict",
    "UserList",
    "Queue",
    "LifoQueue",
    "PriorityQueue",
    "SimpleQueue",
    "WeakValueDictionary",
    "WeakKeyDictionary",
    "WeakSet",
    "Future",
];

/// Whether the value of an assignment in the file at `file` is obvious. `index` resolves the classes that a call
/// names, so that a generic project class called without arguments is not obvious.
pub fn is_obvious(value: &Expr, file: &Path, index: &ClassIndex) -> bool {
    match value {
        Expr::Call(call) => is_obvious_call(call, file, index),
        _ => shape(value).is_some(),
    }
}

/// The kind of a scalar literal. Two literals of different kinds do not share a list or a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Int,
    Float,
    Complex,
    Str,
    Bytes,
    Bool,
}

/// The shape of an obvious literal or container. Two elements of a list, a set, or a dict share one shape when
/// their shapes are equal. A tuple keeps the shape of each of its elements.
#[derive(Debug, PartialEq, Eq)]
enum Shape {
    Scalar(Scalar),
    Tuple(Vec<Shape>),
    List(Box<Shape>),
    Set(Box<Shape>),
    Dict(Box<Shape>, Box<Shape>),
}

/// The shape of a literal or of a non-empty container of literals, and `None` for anything else.
fn shape(expr: &Expr) -> Option<Shape> {
    scalar_kind(expr)
        .map(Shape::Scalar)
        .or_else(|| container_shape(expr))
}

/// The kind of a scalar literal, and `None` for any other expression.
fn scalar_kind(expr: &Expr) -> Option<Scalar> {
    match expr {
        Expr::NumberLiteral(number) => Some(number_kind(&number.value)),
        Expr::UnaryOp(unary) => signed_number_kind(unary),
        _ => text_kind(expr),
    }
}

/// The kind of a string literal, of bytes, or of a bool.
fn text_kind(expr: &Expr) -> Option<Scalar> {
    match expr {
        Expr::StringLiteral(_) | Expr::FString(_) => Some(Scalar::Str),
        _ => bytes_or_bool_kind(expr),
    }
}

fn bytes_or_bool_kind(expr: &Expr) -> Option<Scalar> {
    match expr {
        Expr::BytesLiteral(_) => Some(Scalar::Bytes),
        Expr::BooleanLiteral(_) => Some(Scalar::Bool),
        _ => None,
    }
}

/// The shape of a non-empty tuple, list, set or dict of literals, and `None` for any other expression.
fn container_shape(expr: &Expr) -> Option<Shape> {
    match expr {
        Expr::Tuple(tuple) => tuple_shape(&tuple.elts),
        Expr::Dict(dict) => dict_shape(&dict.items),
        _ => sequence_shape(expr),
    }
}

/// The shape of a non-empty list or set of literals, whose items share one shape.
fn sequence_shape(expr: &Expr) -> Option<Shape> {
    match expr {
        Expr::List(list) => {
            uniform(list.elts.iter().map(shape)).map(|item| Shape::List(Box::new(item)))
        }
        Expr::Set(set) => {
            uniform(set.elts.iter().map(shape)).map(|item| Shape::Set(Box::new(item)))
        }
        _ => None,
    }
}

fn number_kind(number: &Number) -> Scalar {
    match number {
        Number::Int(_) => Scalar::Int,
        Number::Float(_) => Scalar::Float,
        Number::Complex { .. } => Scalar::Complex,
    }
}

/// `-1` and `+2.5`: a sign in front of a numeral. A sign in front of anything else is not a literal.
fn signed_number_kind(unary: &ExprUnaryOp) -> Option<Scalar> {
    match (unary.op, unary.operand.as_ref()) {
        (UnaryOp::UAdd | UnaryOp::USub, Expr::NumberLiteral(number)) => {
            Some(number_kind(&number.value))
        }
        _ => None,
    }
}

/// A tuple whose elements are each obvious. Its elements may differ, but it cannot be empty.
fn tuple_shape(elements: &[Expr]) -> Option<Shape> {
    let shapes = elements.iter().map(shape).collect::<Option<Vec<Shape>>>()?;
    (!shapes.is_empty()).then_some(Shape::Tuple(shapes))
}

/// A dict whose keys share one shape and whose values share one shape. A `**` item has no key, so it is not
/// obvious.
fn dict_shape(items: &[DictItem]) -> Option<Shape> {
    let keys = uniform(items.iter().map(|item| item.key.as_ref().and_then(shape)))?;
    let values = uniform(items.iter().map(|item| shape(&item.value)))?;
    Some(Shape::Dict(Box::new(keys), Box::new(values)))
}

/// The one shape that every item has, when there is at least one item, and all items are obvious and alike.
fn uniform(shapes: impl Iterator<Item = Option<Shape>>) -> Option<Shape> {
    let shapes = shapes.collect::<Option<Vec<Shape>>>()?;
    let first = shapes.first()?;
    if shapes.iter().all(|shape| shape == first) {
        shapes.into_iter().next()
    } else {
        None
    }
}

fn is_obvious_call(call: &ExprCall, file: &Path, index: &ClassIndex) -> bool {
    let Some(name) = callee_name(&call.func) else {
        return false;
    };
    if !is_cap_words(name) {
        return false;
    }
    let takes_arguments = !call.arguments.args.is_empty() || !call.arguments.keywords.is_empty();
    takes_arguments || !is_generic(name, &call.func, file, index)
}

/// The last name of a callee: `User` in `User(...)` and in `models.User(...)`.
fn callee_name(func: &Expr) -> Option<&str> {
    match func {
        Expr::Attribute(attribute) => Some(attribute.attr.as_str()),
        _ => plain_name(func),
    }
}

/// Whether a class with this name, called without arguments, has type parameters that the call leaves unknown.
fn is_generic(name: &str, func: &Expr, file: &Path, index: &ClassIndex) -> bool {
    KNOWN_GENERICS.contains(&name) || index.is_generic_callee(file, func)
}

/// A name in CapWords: after any leading underscores it starts with an uppercase ASCII letter and has a
/// lowercase letter somewhere. So `User` and `Path` are, and `URL`, `MAX` and `datetime` are not.
fn is_cap_words(name: &str) -> bool {
    let body = name.trim_start_matches('_');
    body.starts_with(|c: char| c.is_ascii_uppercase())
        && body.chars().any(|c| c.is_ascii_lowercase())
}
