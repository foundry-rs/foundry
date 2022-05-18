use solang_parser::pt::*;

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
