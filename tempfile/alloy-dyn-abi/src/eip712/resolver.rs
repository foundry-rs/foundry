use crate::{
    eip712::typed_data::Eip712Types, eip712_parser::EncodeType, DynSolType, DynSolValue, Error,
    Result, Specifier,
};
use alloc::{
    borrow::ToOwned,
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};
use alloy_primitives::{keccak256, B256};
use alloy_sol_types::SolStruct;
use core::{cmp::Ordering, fmt};
use parser::{RootType, TypeSpecifier, TypeStem};
use serde::{Deserialize, Deserializer, Serialize};

/// An EIP-712 property definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PropertyDef {
    /// Typename.
    #[serde(rename = "type")]
    type_name: String,
    /// Property Name.
    name: String,
}

impl<'de> Deserialize<'de> for PropertyDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct PropertyDefHelper {
            #[serde(rename = "type")]
            type_name: String,
            name: String,
        }
        let h = PropertyDefHelper::deserialize(deserializer)?;
        Self::new(h.type_name, h.name).map_err(serde::de::Error::custom)
    }
}

impl PropertyDef {
    /// Instantiate a new name-type pair.
    #[inline]
    pub fn new<T, N>(type_name: T, name: N) -> Result<Self>
    where
        T: Into<String>,
        N: Into<String>,
    {
        let type_name = type_name.into();
        TypeSpecifier::parse(type_name.as_str())?;
        Ok(Self::new_unchecked(type_name, name))
    }

    /// Instantiate a new name-type pair, without checking that the type name
    /// is a valid root type.
    #[inline]
    pub fn new_unchecked<T, N>(type_name: T, name: N) -> Self
    where
        T: Into<String>,
        N: Into<String>,
    {
        Self { type_name: type_name.into(), name: name.into() }
    }

    /// Returns the name of the property.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the type name of the property.
    #[inline]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns the root type of the name/type pair, stripping any array.
    #[inline]
    pub fn root_type_name(&self) -> &str {
        self.type_name.split_once('[').map(|t| t.0).unwrap_or(&self.type_name)
    }
}

/// An EIP-712 type definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeDef {
    /// Must always be a ROOT type name with any array stripped.
    type_name: String,
    /// A list of property definitions.
    props: Vec<PropertyDef>,
}

impl Ord for TypeDef {
    // This is not a logic error because we know type names cannot be duplicated in
    // the resolver map
    fn cmp(&self, other: &Self) -> Ordering {
        self.type_name.cmp(&other.type_name)
    }
}

impl PartialOrd for TypeDef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for TypeDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_eip712_encode_type(f)
    }
}

impl TypeDef {
    /// Instantiate a new type definition, checking that the type name is a
    /// valid root type.
    #[inline]
    pub fn new<S: Into<String>>(type_name: S, props: Vec<PropertyDef>) -> Result<Self> {
        let type_name = type_name.into();
        RootType::parse(type_name.as_str())?;
        Ok(Self { type_name, props })
    }

    /// Instantiate a new type definition, without checking that the type name
    /// is a valid root type. This may result in bad behavior in a resolver.
    #[inline]
    pub const fn new_unchecked(type_name: String, props: Vec<PropertyDef>) -> Self {
        Self { type_name, props }
    }

    /// Returns the type name of the type definition.
    #[inline]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns the property definitions of the type definition.
    #[inline]
    pub fn props(&self) -> &[PropertyDef] {
        &self.props
    }

    /// Returns the property names of the type definition.
    #[inline]
    pub fn prop_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.props.iter().map(|p| p.name())
    }

    /// Returns the root property types of the type definition.
    #[inline]
    pub fn prop_root_types(&self) -> impl Iterator<Item = &str> + '_ {
        self.props.iter().map(|p| p.root_type_name())
    }

    /// Returns the property types of the type definition.
    #[inline]
    pub fn prop_types(&self) -> impl Iterator<Item = &str> + '_ {
        self.props.iter().map(|p| p.type_name())
    }

    /// Produces the EIP-712 `encodeType` typestring for this type definition.
    #[inline]
    pub fn eip712_encode_type(&self) -> String {
        let mut s = String::with_capacity(self.type_name.len() + 2 + self.props_bytes_len());
        self.fmt_eip712_encode_type(&mut s).unwrap();
        s
    }

    /// Formats the EIP-712 `encodeType` typestring for this type definition
    /// into `f`.
    pub fn fmt_eip712_encode_type(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str(&self.type_name)?;
        f.write_char('(')?;
        for (i, prop) in self.props.iter().enumerate() {
            if i > 0 {
                f.write_char(',')?;
            }

            f.write_str(prop.type_name())?;
            f.write_char(' ')?;
            f.write_str(prop.name())?;
        }
        f.write_char(')')
    }

    /// Returns the number of bytes that the properties of this type definition
    /// will take up when formatted in the EIP-712 `encodeType` typestring.
    #[inline]
    pub fn props_bytes_len(&self) -> usize {
        self.props.iter().map(|p| p.type_name.len() + p.name.len() + 2).sum()
    }

    /// Return the root type.
    #[inline]
    pub fn root_type(&self) -> RootType<'_> {
        self.type_name.as_str().try_into().expect("checked in instantiation")
    }
}

#[derive(Debug, Default)]
struct DfsContext<'a> {
    visited: BTreeSet<&'a TypeDef>,
    stack: BTreeSet<&'a str>,
}

/// A dependency graph built from the `Eip712Types` object. This is used to
/// safely resolve JSON into a [`crate::DynSolType`] by detecting cycles in the
/// type graph and traversing the dep graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolver {
    /// Nodes in the graph
    // NOTE: Non-duplication of names must be enforced. See note on impl of Ord
    // for TypeDef
    nodes: BTreeMap<String, TypeDef>,
    /// Edges from a type name to its dependencies.
    edges: BTreeMap<String, Vec<String>>,
}

impl Serialize for Resolver {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Eip712Types::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Resolver {
    #[inline]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Eip712Types::deserialize(deserializer).map(Into::into)
    }
}

impl From<Eip712Types> for Resolver {
    fn from(types: Eip712Types) -> Self {
        Self::from(&types)
    }
}

impl From<&Eip712Types> for Resolver {
    #[inline]
    fn from(types: &Eip712Types) -> Self {
        let mut graph = Self::default();
        graph.ingest_types(types);
        graph
    }
}

impl From<&Resolver> for Eip712Types {
    fn from(resolver: &Resolver) -> Self {
        let mut types = Self::default();
        for (name, ty) in &resolver.nodes {
            types.insert(name.clone(), ty.props.clone());
        }
        types
    }
}

impl Resolver {
    /// Instantiate a new resolver from a `SolStruct` type.
    pub fn from_struct<S: SolStruct>() -> Self {
        let mut resolver = Self::default();
        resolver.ingest_sol_struct::<S>();
        resolver
    }

    /// Detect cycles in the subgraph rooted at `type_name`
    fn detect_cycle<'a>(&'a self, type_name: &str, context: &mut DfsContext<'a>) -> bool {
        let ty = match self.nodes.get(type_name) {
            Some(ty) => ty,
            None => return false,
        };

        if context.stack.contains(type_name) {
            return true;
        }
        if context.visited.contains(ty) {
            return false;
        }

        // update visited and stack
        context.visited.insert(ty);
        context.stack.insert(&ty.type_name);

        if self
            .edges
            .get(&ty.type_name)
            .unwrap()
            .iter()
            .any(|edge| self.detect_cycle(edge, context))
        {
            return true;
        }

        context.stack.remove(type_name);
        false
    }

    /// Ingest types from an EIP-712 `encodeType`.
    pub fn ingest_string(&mut self, s: impl AsRef<str>) -> Result<()> {
        let encode_type: EncodeType<'_> = s.as_ref().try_into()?;
        for t in encode_type.types {
            self.ingest(t.to_owned());
        }
        Ok(())
    }

    /// Ingest a sol struct typedef.
    pub fn ingest_sol_struct<S: SolStruct>(&mut self) {
        self.ingest_string(S::eip712_encode_type()).unwrap();
    }

    /// Ingest a type.
    pub fn ingest(&mut self, type_def: TypeDef) {
        let type_name = type_def.type_name.to_owned();
        // Insert the edges into the graph
        {
            let entry = self.edges.entry(type_name.clone()).or_default();
            for prop in &type_def.props {
                entry.push(prop.root_type_name().to_owned());
            }
        } // entry dropped here

        // Insert the node into the graph
        self.nodes.insert(type_name, type_def);
    }

    /// Ingest a `Types` object into the resolver, discarding any invalid types.
    pub fn ingest_types(&mut self, types: &Eip712Types) {
        for (type_name, props) in types {
            if let Ok(ty) = TypeDef::new(type_name.clone(), props.to_vec()) {
                self.ingest(ty);
            }
        }
    }

    // This function assumes that the graph is acyclic.
    fn linearize_into<'a>(
        &'a self,
        resolution: &mut Vec<&'a TypeDef>,
        root_type: RootType<'_>,
    ) -> Result<()> {
        if root_type.try_basic_solidity().is_ok() {
            return Ok(());
        }

        let this_type = self
            .nodes
            .get(root_type.span())
            .ok_or_else(|| Error::missing_type(root_type.span()))?;

        let edges: &Vec<String> = self.edges.get(root_type.span()).unwrap();

        if !resolution.contains(&this_type) {
            resolution.push(this_type);
            for edge in edges {
                let rt = edge.as_str().try_into()?;
                self.linearize_into(resolution, rt)?;
            }
        }

        Ok(())
    }

    /// This function linearizes a type into a list of typedefs of its
    /// dependencies.
    pub fn linearize(&self, type_name: &str) -> Result<Vec<&TypeDef>> {
        let mut context = DfsContext::default();
        if self.detect_cycle(type_name, &mut context) {
            return Err(Error::circular_dependency(type_name));
        }
        let root_type = type_name.try_into()?;
        let mut resolution = vec![];
        self.linearize_into(&mut resolution, root_type)?;
        Ok(resolution)
    }

    /// Resolve a typename into a [`crate::DynSolType`] or return an error if
    /// the type is missing, or contains a circular dependency.
    pub fn resolve(&self, type_name: &str) -> Result<DynSolType> {
        if self.detect_cycle(type_name, &mut Default::default()) {
            return Err(Error::circular_dependency(type_name));
        }
        self.unchecked_resolve(&type_name.try_into()?)
    }

    /// Resolve a type into a [`crate::DynSolType`] without checking for cycles.
    fn unchecked_resolve(&self, type_spec: &TypeSpecifier<'_>) -> Result<DynSolType> {
        let ty = match &type_spec.stem {
            TypeStem::Root(root) => self.resolve_root_type(*root),
            TypeStem::Tuple(tuple) => tuple
                .types
                .iter()
                .map(|ty| self.unchecked_resolve(ty))
                .collect::<Result<_, _>>()
                .map(DynSolType::Tuple),
        }?;
        Ok(ty.array_wrap_from_iter(type_spec.sizes.iter().copied()))
    }

    /// Resolves a root Solidity type into either a basic type or a custom
    /// struct.
    fn resolve_root_type(&self, root_type: RootType<'_>) -> Result<DynSolType> {
        if let Ok(ty) = root_type.resolve() {
            return Ok(ty);
        }

        let ty = self
            .nodes
            .get(root_type.span())
            .ok_or_else(|| Error::missing_type(root_type.span()))?;

        let prop_names: Vec<_> = ty.prop_names().map(str::to_string).collect();
        let tuple: Vec<_> = ty
            .prop_types()
            .map(|ty| self.unchecked_resolve(&ty.try_into()?))
            .collect::<Result<_, _>>()?;

        Ok(DynSolType::CustomStruct { name: ty.type_name.clone(), prop_names, tuple })
    }

    /// Encode the type into an EIP-712 `encodeType` string
    ///
    /// <https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype>
    pub fn encode_type(&self, name: &str) -> Result<String> {
        let linear = self.linearize(name)?;
        let first = linear.first().unwrap().eip712_encode_type();

        // Sort references by name (eip-712 encodeType spec)
        let mut sorted_refs =
            linear[1..].iter().map(|t| t.eip712_encode_type()).collect::<Vec<String>>();
        sorted_refs.sort();

        Ok(sorted_refs.iter().fold(first, |mut acc, s| {
            acc.push_str(s);
            acc
        }))
    }

    /// Compute the keccak256 hash of the EIP-712 `encodeType` string.
    pub fn type_hash(&self, name: &str) -> Result<B256> {
        self.encode_type(name).map(keccak256)
    }

    /// Encode the data according to EIP-712 `encodeData` rules.
    pub fn encode_data(&self, value: &DynSolValue) -> Result<Option<Vec<u8>>> {
        Ok(match value {
            DynSolValue::CustomStruct { tuple: inner, .. }
            | DynSolValue::Array(inner)
            | DynSolValue::FixedArray(inner) => {
                let mut bytes = Vec::with_capacity(inner.len() * 32);
                for v in inner {
                    bytes.extend(self.eip712_data_word(v)?.as_slice());
                }
                Some(bytes)
            }
            DynSolValue::Bytes(buf) => Some(buf.to_vec()),
            DynSolValue::String(s) => Some(s.as_bytes().to_vec()),
            _ => None,
        })
    }

    /// Encode the data as a struct property according to EIP-712 `encodeData`
    /// rules. Atomic types are encoded as-is, while non-atomic types are
    /// encoded as their `encodeData` hash.
    pub fn eip712_data_word(&self, value: &DynSolValue) -> Result<B256> {
        if let Some(word) = value.as_word() {
            return Ok(word);
        }

        let mut bytes;
        let to_hash = match value {
            DynSolValue::CustomStruct { name, tuple, .. } => {
                bytes = self.type_hash(name)?.to_vec();
                for v in tuple {
                    bytes.extend(self.eip712_data_word(v)?.as_slice());
                }
                &bytes[..]
            }
            DynSolValue::Array(inner) | DynSolValue::FixedArray(inner) => {
                bytes = Vec::with_capacity(inner.len() * 32);
                for v in inner {
                    bytes.extend(self.eip712_data_word(v)?);
                }
                &bytes[..]
            }
            DynSolValue::Bytes(buf) => buf,
            DynSolValue::String(s) => s.as_bytes(),
            _ => unreachable!("all types are words or covered in the match"),
        };
        Ok(keccak256(to_hash))
    }

    /// Check if the resolver graph contains a type by its name.
    ///
    /// ## Warning
    ///
    /// This checks by NAME only. It does NOT check for type
    pub fn contains_type_name(&self, name: &str) -> bool {
        self.nodes.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloy_sol_types::sol;

    #[test]
    fn it_detects_cycles() {
        let mut graph = Resolver::default();
        graph.ingest(TypeDef::new_unchecked(
            "A".to_string(),
            vec![PropertyDef::new_unchecked("B", "myB")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "B".to_string(),
            vec![PropertyDef::new_unchecked("C", "myC")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "C".to_string(),
            vec![PropertyDef::new_unchecked("A", "myA")],
        ));

        assert!(graph.detect_cycle("A", &mut DfsContext::default()));
    }

    #[test]
    fn it_produces_encode_type_strings() {
        let mut graph = Resolver::default();
        graph.ingest(TypeDef::new_unchecked(
            "A".to_string(),
            vec![PropertyDef::new_unchecked("C", "myC"), PropertyDef::new_unchecked("B", "myB")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "B".to_string(),
            vec![PropertyDef::new_unchecked("C", "myC")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "C".to_string(),
            vec![
                PropertyDef::new_unchecked("uint256", "myUint"),
                PropertyDef::new_unchecked("uint256", "myUint2"),
            ],
        ));

        // This tests specific adherence to EIP-712 specified ordering.
        // Referenced types are sorted by name, the Primary type is at the
        // start of the string
        assert_eq!(
            graph.encode_type("A").unwrap(),
            "A(C myC,B myB)B(C myC)C(uint256 myUint,uint256 myUint2)"
        );
    }

    #[test]
    fn it_resolves_types() {
        let mut graph = Resolver::default();
        graph.ingest(TypeDef::new_unchecked(
            "A".to_string(),
            vec![PropertyDef::new_unchecked("B", "myB")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "B".to_string(),
            vec![PropertyDef::new_unchecked("C", "myC")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "C".to_string(),
            vec![PropertyDef::new_unchecked("uint256", "myUint")],
        ));

        let c = DynSolType::CustomStruct {
            name: "C".to_string(),
            prop_names: vec!["myUint".to_string()],
            tuple: vec![DynSolType::Uint(256)],
        };
        let b = DynSolType::CustomStruct {
            name: "B".to_string(),
            prop_names: vec!["myC".to_string()],
            tuple: vec![c.clone()],
        };
        let a = DynSolType::CustomStruct {
            name: "A".to_string(),
            prop_names: vec!["myB".to_string()],
            tuple: vec![b.clone()],
        };
        assert_eq!(graph.resolve("A"), Ok(a));
        assert_eq!(graph.resolve("B"), Ok(b));
        assert_eq!(graph.resolve("C"), Ok(c));
    }

    #[test]
    fn it_resolves_types_with_arrays() {
        let mut graph = Resolver::default();
        graph.ingest(TypeDef::new_unchecked(
            "A".to_string(),
            vec![PropertyDef::new_unchecked("B", "myB")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "B".to_string(),
            vec![PropertyDef::new_unchecked("C[]", "myC")],
        ));
        graph.ingest(TypeDef::new_unchecked(
            "C".to_string(),
            vec![PropertyDef::new_unchecked("uint256", "myUint")],
        ));

        let c = DynSolType::CustomStruct {
            name: "C".to_string(),
            prop_names: vec!["myUint".to_string()],
            tuple: vec![DynSolType::Uint(256)],
        };
        let b = DynSolType::CustomStruct {
            name: "B".to_string(),
            prop_names: vec!["myC".to_string()],
            tuple: vec![DynSolType::Array(Box::new(c.clone()))],
        };
        let a = DynSolType::CustomStruct {
            name: "A".to_string(),
            prop_names: vec!["myB".to_string()],
            tuple: vec![b.clone()],
        };
        assert_eq!(graph.resolve("C"), Ok(c));
        assert_eq!(graph.resolve("B"), Ok(b));
        assert_eq!(graph.resolve("A"), Ok(a));
    }

    #[test]
    fn encode_type_round_trip() {
        const ENCODE_TYPE: &str = "A(C myC,B myB)B(C myC)C(uint256 myUint,uint256 myUint2)";
        let mut graph = Resolver::default();
        graph.ingest_string(ENCODE_TYPE).unwrap();
        assert_eq!(graph.encode_type("A").unwrap(), ENCODE_TYPE);

        const ENCODE_TYPE_2: &str = "Transaction(Person from,Person to,Asset tx)Asset(address token,uint256 amount)Person(address wallet,string name)";
        let mut graph = Resolver::default();
        graph.ingest_string(ENCODE_TYPE_2).unwrap();
        assert_eq!(graph.encode_type("Transaction").unwrap(), ENCODE_TYPE_2);
    }

    #[test]
    fn it_ingests_sol_structs() {
        sol!(
            struct MyStruct {
                uint256 a;
            }
        );

        let mut graph = Resolver::default();
        graph.ingest_sol_struct::<MyStruct>();
        assert_eq!(graph.encode_type("MyStruct").unwrap(), MyStruct::eip712_encode_type());
    }
}
