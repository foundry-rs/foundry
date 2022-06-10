use solang_parser::pt::*;

/// Check if the namespace matches another. This should be removed by a future version of
/// solang_parser which implements IdentifierPath's directly
pub fn namespace_matches(left: &Expression, right: &Expression) -> bool {
    match (left, right) {
        (
            Expression::MemberAccess(_, left_namespace, left_ident),
            Expression::MemberAccess(_, right_namespace, right_ident),
        ) => {
            left_ident.name == right_ident.name &&
                namespace_matches(left_namespace, right_namespace)
        }
        (Expression::Variable(left_ident), Expression::Variable(right_ident)) => {
            left_ident.name == right_ident.name
        }
        _ => false,
    }
}
