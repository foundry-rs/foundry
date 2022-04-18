use solang_parser::pt;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

#[derive(Debug)]
pub struct FunctionAttribute(pub pt::FunctionAttribute);

impl PartialEq<Self> for FunctionAttribute {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (
                pt::FunctionAttribute::Mutability(mutability1),
                pt::FunctionAttribute::Mutability(mutability2),
            ) => matches!(
                (mutability1, mutability2),
                (pt::Mutability::Pure(_), pt::Mutability::Pure(_)) |
                    (pt::Mutability::View(_), pt::Mutability::View(_)) |
                    (pt::Mutability::Constant(_), pt::Mutability::Constant(_)) |
                    (pt::Mutability::Payable(_), pt::Mutability::Payable(_))
            ),
            (
                pt::FunctionAttribute::Visibility(visibility1),
                pt::FunctionAttribute::Visibility(visibility2),
            ) => matches!(
                (visibility1, visibility2),
                (pt::Visibility::External(_), pt::Visibility::External(_)) |
                    (pt::Visibility::Public(_), pt::Visibility::Public(_)) |
                    (pt::Visibility::Internal(_), pt::Visibility::Internal(_)) |
                    (pt::Visibility::Private(_), pt::Visibility::Private(_))
            ),
            (pt::FunctionAttribute::Virtual(_), pt::FunctionAttribute::Virtual(_)) => true,
            (
                pt::FunctionAttribute::Override(_, bases1),
                pt::FunctionAttribute::Override(_, bases2),
            ) => {
                fn elements_set<T>(items: impl Iterator<Item = T>) -> HashMap<T, usize>
                where
                    T: Eq + Hash,
                {
                    items.fold(HashMap::new(), |mut map, item| {
                        *map.entry(item).or_insert(0) += 1;
                        map
                    })
                }

                elements_set(bases1.iter().map(|ident| &ident.name)) ==
                    elements_set(bases2.iter().map(|ident| &ident.name))
            }
            // We need to compare bases' arguments but they are arbitrary expressions, so for now
            // let's say that all bases are different.
            (
                pt::FunctionAttribute::BaseOrModifier(_, _base1),
                pt::FunctionAttribute::BaseOrModifier(_, _base2),
            ) => false,
            _ => false,
        }
    }
}

impl Eq for FunctionAttribute {}

impl Hash for FunctionAttribute {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            pt::FunctionAttribute::Mutability(mutability) => format!("{mutability}").hash(state),
            pt::FunctionAttribute::Visibility(visibility) => format!("{visibility}").hash(state),
            pt::FunctionAttribute::Virtual(_) => "virtual".hash(state),
            pt::FunctionAttribute::Override(_, bases) => {
                bases.iter().map(|identifier| &identifier.name).collect::<Vec<_>>().hash(state)
            }
            pt::FunctionAttribute::BaseOrModifier(_, base) => base.name.name.hash(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pt_ext::FunctionAttribute;
    use solang_parser::pt;

    #[test]
    fn test_function_attribute_eq() {
        let identifier1 = pt::Identifier { loc: pt::Loc::Builtin, name: "first".to_string() };
        let identifier2 = pt::Identifier { loc: pt::Loc::Implicit, name: "second".to_string() };

        let attribute1 = FunctionAttribute(pt::FunctionAttribute::Override(
            pt::Loc::Builtin,
            vec![identifier1.clone(), identifier2.clone(), identifier1.clone()],
        ));
        let attribute2 = FunctionAttribute(pt::FunctionAttribute::Override(
            pt::Loc::Builtin,
            vec![identifier2.clone(), identifier1.clone(), identifier1.clone()],
        ));

        assert_eq!(attribute1, attribute2)
    }
}
