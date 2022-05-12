use solang_parser::pt::*;

pub fn namespace_matches(left: &Expression, right: &Expression) -> bool {
    if matches!(left, Expression::MemberAccess(..)) || matches!(left, Expression::MemberAccess(..))
    {
        // TODO handle me
        todo!()
    }
    match (left, right) {
        (Expression::Variable(left_ident), Expression::Variable(right_ident)) => {
            left_ident.name == right_ident.name
        }
        _ => false,
    }
}
