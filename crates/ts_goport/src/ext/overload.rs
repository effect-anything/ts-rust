//! Operator types for check and LSP.
//!
//! The author's `+` is not rewritten before check. When either operand's
//! type has `Symbol.operatorPlus` (and the same for `- * / % **`), the
//! expression type is that call signature's return type. The left operand
//! is tried first. `Symbol.deferOperation` tries the right operand. A pair
//! with no signature falls through to TypeScript's own operator.
//!
//! This is tsover's `checkBinaryLikeExpressionWorker` hook. Emit still
//! prints `__tsover_*`.

use crate::ast::{get_source_file_of_node, is_function_like_declaration, is_prologue_directive};
use crate::astdata::SyntaxKind;
use crate::checker::Checker;
use crate::core::{Node, TypeId};
use crate::flags::{SignatureKind, TypeFlags};

/// The binary expression to show when hover is on an overloaded operator.
///
/// Stock quickinfo ignores a `+` token. In a `"use tsover"` or `"use gpu"`
/// scope the expression type is the overload result, so hover should name it.
pub fn hover_expression(node: Node) -> Option<Node> {
    let expr = if node.kind() == SyntaxKind::BinaryExpression {
        node
    } else {
        let parent = node.parent();
        if parent.kind() == SyntaxKind::BinaryExpression && parent.operator_token() == node {
            parent
        } else {
            return None;
        }
    };
    in_overload_scope(expr).then_some(expr)
}

/// The type of `left operator right`, when an overload symbol applies.
pub fn binary_type(
    checker: &mut Checker,
    left: Node,
    operator: SyntaxKind,
    right: Node,
    left_type: TypeId,
    right_type: TypeId,
) -> Option<TypeId> {
    let symbol = symbol_name(operator)?;
    if !in_overload_scope(left) {
        return None;
    }
    let property = checker.get_property_name_for_known_symbol_name(symbol);
    let defer = defer_operation_type(checker);
    let pairs = pairs(checker, left_type, right_type);
    let mut results = Vec::with_capacity(pairs.len());
    for (lhs, rhs) in pairs {
        let mut result = signature_return(checker, lhs, &property, lhs, rhs);
        let left_missing = result.is_none();
        if left_missing || is_defer(checker, result, defer) {
            result = signature_return(checker, rhs, &property, lhs, rhs);
        }
        if is_defer(checker, result, defer) {
            result = None;
        }
        let Some(result) = result else {
            return None;
        };
        results.push(result);
    }
    if results.is_empty() {
        return None;
    }
    if results.len() == 1 {
        return Some(results[0]);
    }
    Some(checker.get_union_type(&results))
}

fn symbol_name(operator: SyntaxKind) -> Option<&'static str> {
    let base = match operator {
        SyntaxKind::PlusEqualsToken => SyntaxKind::PlusToken,
        SyntaxKind::MinusEqualsToken => SyntaxKind::MinusToken,
        SyntaxKind::AsteriskEqualsToken => SyntaxKind::AsteriskToken,
        SyntaxKind::SlashEqualsToken => SyntaxKind::SlashToken,
        SyntaxKind::AsteriskAsteriskEqualsToken => SyntaxKind::AsteriskAsteriskToken,
        SyntaxKind::PercentEqualsToken => SyntaxKind::PercentToken,
        other => other,
    };
    match base {
        SyntaxKind::PlusToken => Some("operatorPlus"),
        SyntaxKind::MinusToken => Some("operatorMinus"),
        SyntaxKind::AsteriskToken => Some("operatorStar"),
        SyntaxKind::SlashToken => Some("operatorSlash"),
        SyntaxKind::AsteriskAsteriskToken => Some("operatorStarStar"),
        SyntaxKind::PercentToken => Some("operatorPercent"),
        _ => None,
    }
}

fn in_overload_scope(node: Node) -> bool {
    let file = get_source_file_of_node(node);
    if file.is_some()
        && (prologue_has(file.statements(), "use tsover")
            || prologue_has(file.statements(), "use gpu"))
    {
        return true;
    }
    let mut current = node;
    while current.is_some() {
        if is_function_like_declaration(current) {
            let body = current.body();
            if body.kind() == SyntaxKind::Block
                && (prologue_has(body.statements(), "use gpu")
                    || prologue_has(body.statements(), "use tsover"))
            {
                return true;
            }
        }
        current = current.parent();
    }
    false
}

fn prologue_has(statements: impl IntoIterator<Item = Node>, text: &str) -> bool {
    for statement in statements {
        if !is_prologue_directive(statement) {
            return false;
        }
        if super::tsover::is_named_directive(
            statement.expression(),
            text.strip_prefix("use ").unwrap_or(text),
        ) {
            return true;
        }
    }
    false
}

fn pairs(checker: &mut Checker, left: TypeId, right: TypeId) -> Vec<(TypeId, TypeId)> {
    let left_members = members(checker, left);
    let right_members = members(checker, right);
    if left_members.len() == 1 && right_members.len() == 1 {
        return vec![(left_members[0], right_members[0])];
    }
    let mut out = Vec::new();
    for lhs in &left_members {
        for rhs in &right_members {
            out.push((*lhs, *rhs));
        }
    }
    out
}

fn members(checker: &mut Checker, t: TypeId) -> Vec<TypeId> {
    let flags = checker.ty(t).flags;
    if flags.intersects(TypeFlags::UNION) {
        let types = checker
            .ty(t)
            .as_union_or_intersection_type()
            .types()
            .to_vec();
        return types
            .into_iter()
            .flat_map(|part| members(checker, part))
            .collect();
    }
    if flags.intersects(TypeFlags::TYPE_VARIABLE) {
        let constraint = checker.get_base_constraint_of_type(t);
        if constraint.is_some() && constraint != t {
            return members(checker, constraint);
        }
    }
    vec![t]
}

fn signature_return(
    checker: &mut Checker,
    operand: TypeId,
    property: &str,
    left: TypeId,
    right: TypeId,
) -> Option<TypeId> {
    let property_type = checker.get_type_of_property_of_type(operand, property);
    if !property_type.is_some() {
        return None;
    }
    let signatures: Vec<_> = checker
        .get_signatures_of_type(property_type, SignatureKind::CALL)
        .iter()
        .copied()
        .collect();
    for signature in signatures {
        let first = checker.get_type_at_position(signature, 0);
        let second = checker.get_type_at_position(signature, 1);
        if checker.is_type_assignable_to(left, first)
            && checker.is_type_assignable_to(right, second)
        {
            return Some(checker.get_return_type_of_signature(signature));
        }
    }
    None
}

fn defer_operation_type(checker: &mut Checker) -> TypeId {
    let constructor = checker.get_global_es_symbol_constructor_symbol_or_nil();
    if !constructor.is_some() {
        return TypeId::NIL;
    }
    let constructor_type = checker.get_type_of_symbol(constructor);
    checker.get_type_of_property_of_type(constructor_type, "deferOperation")
}

fn is_defer(checker: &mut Checker, result: Option<TypeId>, defer: TypeId) -> bool {
    match result {
        Some(result) if defer.is_some() => checker.is_type_identical_to(result, defer),
        _ => false,
    }
}
