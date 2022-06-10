use solang_parser::pt::*;

pub fn namespace_matches(left: &IdentifierPath, right: &IdentifierPath) -> bool {
    left.identifiers.iter().zip(right.identifiers.iter()).all(|(l, r)| l.name == r.name)
}
