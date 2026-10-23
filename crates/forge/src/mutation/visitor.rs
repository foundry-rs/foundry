use crate::mutation::{
    mutant::{Mutant, OwnedLiteral},
    mutators::{MutationContext, mutator_registry::MutatorRegistry},
    type_analysis::{AssignmentReplacement, MutationExclusion, MutationExclusionSet},
};
use eyre::Report;
use foundry_config::MutatorType;
use solar::ast::{Expr, ItemContract, ItemFunction, VariableDefinition, visit::Visit, yul};
use std::{ops::ControlFlow, path::PathBuf};

#[cfg(test)]
use crate::mutation::mutators::Mutator;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AssignVarTypes {
    Literal(OwnedLiteral),
    Identifier(String),
    NegatedIdentifier(String),
}

/// A visitor which collect all expression to mutate as well as the mutation types
#[allow(clippy::type_complexity)]
pub struct MutantVisitor<'src> {
    pub mutation_to_conduct: Vec<Mutant>,
    errors: Vec<Report>,
    pub mutator_registry: MutatorRegistry,
    pub path: PathBuf,
    pub source: Option<&'src str>,
    /// Optional per-contract name filter. When `Some`, mutations are only collected
    /// from contracts whose name matches the predicate.
    pub contract_filter: Option<Box<dyn Fn(&str) -> bool>>,
    /// Whether the currently-visited contract is allowed by `contract_filter`.
    /// `true` when no filter is set or when we are visiting a contract whose name
    /// matched the filter. Top-level items (outside any contract) are always
    /// considered "allowed".
    in_allowed_contract: bool,
    mutation_exclusions: MutationExclusionSet,
}

impl<'src> MutantVisitor<'src> {
    /// Create a visitor with the specified mutator operators enabled
    pub fn with_operators(path: PathBuf, operators: &[MutatorType]) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            errors: Vec::new(),
            mutator_registry: MutatorRegistry::from_enabled(operators),
            path,
            source: None,
            contract_filter: None,
            in_allowed_contract: true,
            mutation_exclusions: MutationExclusionSet::new(),
        }
    }

    /// Use only a set of mutators
    #[cfg(test)]
    pub fn new_with_mutators(path: PathBuf, mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self {
            mutator_registry: MutatorRegistry::new_with_mutators(mutators),
            ..Self::with_operators(path, &[])
        }
    }

    /// Set the source code for extracting original text
    pub const fn with_source(mut self, source: &'src str) -> Self {
        self.source = Some(source);
        self
    }

    /// Set a contract-name filter; only contracts whose name matches the
    /// predicate will have their bodies mutated.
    pub fn with_contract_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&str) -> bool + 'static,
    {
        self.contract_filter = Some(Box::new(filter));
        self
    }

    /// Exclude operator replacements rejected by type analysis.
    pub fn with_mutation_exclusions(mut self, mutations: MutationExclusionSet) -> Self {
        self.mutation_exclusions = mutations;
        self
    }

    pub fn take_errors(&mut self) -> Vec<Report> {
        std::mem::take(&mut self.errors)
    }

    fn collect_mutations(&mut self, context: &MutationContext<'_>) {
        let result = self.mutator_registry.generate_mutations(context);
        self.mutation_to_conduct.extend(result.mutations.into_iter().filter(|mutant| {
            let exclusion = match &mutant.mutation {
                crate::mutation::mutant::MutationType::Assignment(kind) => {
                    let replacement = match kind {
                        AssignVarTypes::Literal(OwnedLiteral::Number(value)) if value.is_zero() => {
                            AssignmentReplacement::Zero
                        }
                        AssignVarTypes::Literal(OwnedLiteral::NegatedNumber(_))
                        | AssignVarTypes::NegatedIdentifier(_) => AssignmentReplacement::Negate,
                        _ => return true,
                    };
                    MutationExclusion::assignment(mutant.span, replacement)
                }
                crate::mutation::mutant::MutationType::BinaryOpExpr { new_op, .. } => {
                    MutationExclusion::binary(mutant.span, *new_op)
                }
                crate::mutation::mutant::MutationType::UnaryOperator(unary) => {
                    MutationExclusion::unary(mutant.span, unary.resulting_op_kind)
                }
                _ => return true,
            };
            !self.mutation_exclusions.contains(&exclusion)
        }));

        for err in result.errors {
            self.errors.push(err.wrap_err(format!(
                "failed to generate mutations for {}:{}:{}",
                self.path.display(),
                context.line_number(),
                context.column_number()
            )));
        }
    }
}

impl<'ast> Visit<'ast> for MutantVisitor<'ast> {
    type BreakValue = ();

    fn visit_item_contract(
        &mut self,
        contract: &'ast ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // When a contract name filter is configured, only descend into matching
        // contracts. We toggle `in_allowed_contract` for the duration of the
        // walk so nested visit_expr / visit_variable_definition calls can gate
        // mutant collection accordingly.
        let prev = self.in_allowed_contract;
        self.in_allowed_contract = match &self.contract_filter {
            Some(filter) => filter(contract.name.as_str()),
            None => true,
        };
        let res = self.walk_item_contract(contract);
        self.in_allowed_contract = prev;
        res
    }

    fn visit_item_function(
        &mut self,
        function: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if self.in_allowed_contract && function.body.is_some() {
            let mut builder = MutationContext::builder()
                .with_path(self.path.clone())
                .with_span(function.body_span)
                .with_function(function);
            if let Some(src) = self.source {
                builder = builder.with_source(src);
            }
            self.collect_mutations(&builder.build().expect("function body context"));
        }
        self.walk_item_function(function)
    }

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
            return self.walk_variable_definition(var);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(var.span)
            .with_var_definition(var);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context = builder
            .build()
            .expect("MutationContext requires both path and span for variable definition");

        self.collect_mutations(&context);
        self.walk_variable_definition(var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
            return self.walk_expr(expr);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(expr.span)
            .with_expr(expr);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context =
            builder.build().expect("MutationContext requires both path and span for expression");

        self.collect_mutations(&context);
        self.walk_expr(expr)
    }

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
            return self.walk_yul_expr(expr);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(expr.span)
            .with_yul_expr(expr);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context = builder
            .build()
            .expect("MutationContext requires both path and span for yul expression");

        self.collect_mutations(&context);
        self.walk_yul_expr(expr)
    }
}

#[cfg(test)]
mod tests {
    use eyre::{Result, eyre};
    use solar::{
        ast::{Arena, interface::source_map::FileName},
        parse::Parser,
    };

    use super::*;
    use crate::mutation::{Session, mutant::MutationType};

    struct FailingExprMutator;

    impl Mutator for FailingExprMutator {
        fn generate_mutants(&self, _ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
            Err(eyre!("synthetic visitor failure"))
        }

        fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
            ctxt.expr.is_some()
        }
    }

    struct PassingExprMutator;

    impl Mutator for PassingExprMutator {
        fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
            Ok(vec![Mutant {
                path: ctxt.path.clone(),
                span: ctxt.span,
                mutation: MutationType::DeleteExpression,
                original: ctxt.original_text(),
                source_line: ctxt.source_line(),
                line_number: ctxt.line_number(),
                column_number: ctxt.column_number(),
            }])
        }

        fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
            ctxt.expr.is_some()
        }
    }

    #[test]
    fn visitor_collects_mutations_and_surfaces_mutator_errors() {
        let source = "\
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Test {
    function test() public {
        uint256 x = 1 + 2;
    }
}
";
        let path = PathBuf::from("test.sol");
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::new_with_mutators(
                path,
                vec![Box::new(FailingExprMutator), Box::new(PassingExprMutator)],
            )
            .with_source(source);

            let _ = visitor.visit_source_unit(&ast);
            let errors = visitor.take_errors();

            assert!(!visitor.mutation_to_conduct.is_empty());
            assert!(!errors.is_empty());

            let err = format!("{:?}", errors[0]);
            assert!(err.contains("failed to generate mutations for test.sol:"));
            assert!(err.contains("synthetic visitor failure"));
        });
    }

    #[test]
    fn visitor_excludes_binary_mutations_rejected_by_type_analysis() {
        let source = "\
pragma solidity ^0.8.0;
contract Test {
    function check(uint256 x) public pure returns (bool) {
        return x == 0;
    }
}
";
        let path = PathBuf::from("test.sol");
        let lo = source.find("x == 0").unwrap() as u32;
        let span = solar::ast::Span::new(
            solar::interface::BytePos(lo),
            solar::interface::BytePos(lo + "x == 0".len() as u32),
        );
        let mutation_exclusions =
            [MutationExclusion::binary(span, solar::ast::BinOpKind::Le)].into_iter().collect();
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::with_operators(path, &MutatorType::all())
                .with_source(source)
                .with_mutation_exclusions(mutation_exclusions);

            let _ = visitor.visit_source_unit(&ast);
            let comparison_mutations = visitor.mutation_to_conduct.iter().filter_map(|mutant| {
                let MutationType::BinaryOpExpr { new_op, .. } = mutant.mutation else {
                    return None;
                };
                (mutant.span == span).then_some(new_op)
            });
            let comparison_mutations = comparison_mutations.collect::<Vec<_>>();

            assert!(!comparison_mutations.contains(&solar::ast::BinOpKind::Le));
            assert!(comparison_mutations.contains(&solar::ast::BinOpKind::Lt));
        });
    }

    #[test]
    fn visitor_excludes_assignment_mutations_rejected_by_type_analysis() {
        let source = "\
pragma solidity ^0.8.0;
contract Test {
    function check(address account) public pure {
        address copy = account;
    }
}
";
        let path = PathBuf::from("test.sol");
        let lo = source.rfind("account").unwrap() as u32;
        let span = solar::ast::Span::new(
            solar::interface::BytePos(lo),
            solar::interface::BytePos(lo + "account".len() as u32),
        );
        let mutation_exclusions = [
            MutationExclusion::assignment(span, AssignmentReplacement::Zero),
            MutationExclusion::assignment(span, AssignmentReplacement::Negate),
        ]
        .into_iter()
        .collect();
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::new_with_mutators(
                path,
                vec![Box::new(crate::mutation::mutators::assignment_mutator::AssignmentMutator)],
            )
            .with_source(source)
            .with_mutation_exclusions(mutation_exclusions);

            let _ = visitor.visit_source_unit(&ast);

            assert!(visitor.mutation_to_conduct.iter().all(|mutant| mutant.span != span));
        });
    }

    #[test]
    fn visitor_excludes_negated_number_but_retains_zero_assignment() {
        let source = "\
pragma solidity ^0.8.0;
contract Test {
    function check() public pure {
        uint8 copy = 1;
    }
}
";
        let path = PathBuf::from("test.sol");
        let lo = source.rfind('1').unwrap() as u32;
        let span =
            solar::ast::Span::new(solar::interface::BytePos(lo), solar::interface::BytePos(lo + 1));
        let mutation_exclusions =
            [MutationExclusion::assignment(span, AssignmentReplacement::Negate)]
                .into_iter()
                .collect();
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::new_with_mutators(
                path,
                vec![Box::new(crate::mutation::mutators::assignment_mutator::AssignmentMutator)],
            )
            .with_source(source)
            .with_mutation_exclusions(mutation_exclusions);

            let _ = visitor.visit_source_unit(&ast);
            let replacements = visitor
                .mutation_to_conduct
                .iter()
                .filter(|mutant| mutant.span == span)
                .map(|mutant| mutant.mutation.to_string())
                .collect::<Vec<_>>();

            assert_eq!(replacements, ["0"]);
        });
    }

    #[test]
    fn visitor_excludes_unsigned_negation_but_retains_unary_swaps() {
        let source = "\
pragma solidity ^0.8.0;
contract Test {
    function increment(uint256 x) public pure {
        x++;
    }
}
";
        let path = PathBuf::from("test.sol");
        let lo = source.find("x++").unwrap() as u32;
        let span = solar::ast::Span::new(
            solar::interface::BytePos(lo),
            solar::interface::BytePos(lo + "x++".len() as u32),
        );
        let mutation_exclusions =
            [MutationExclusion::unary(span, solar::ast::UnOpKind::Neg)].into_iter().collect();
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::new_with_mutators(
                path,
                vec![Box::new(crate::mutation::mutators::unary_op_mutator::UnaryOpMutator)],
            )
            .with_source(source)
            .with_mutation_exclusions(mutation_exclusions);

            let _ = visitor.visit_source_unit(&ast);
            let unary_mutations = visitor
                .mutation_to_conduct
                .iter()
                .filter_map(|mutant| {
                    let MutationType::UnaryOperator(unary) = &mutant.mutation else {
                        return None;
                    };
                    (mutant.span == span).then_some(unary.resulting_op_kind)
                })
                .collect::<Vec<_>>();

            assert_eq!(unary_mutations.len(), 4);
            assert!(!unary_mutations.contains(&solar::ast::UnOpKind::Neg));
            assert!(unary_mutations.contains(&solar::ast::UnOpKind::PreInc));
            assert!(unary_mutations.contains(&solar::ast::UnOpKind::PreDec));
            assert!(unary_mutations.contains(&solar::ast::UnOpKind::BitNot));
            assert!(unary_mutations.contains(&solar::ast::UnOpKind::PostDec));
        });
    }

    #[test]
    fn function_body_replacements_preserve_headers_and_skip_unsupported_functions() {
        let source = r#"
abstract contract Target {
    constructor() { revert(); }
    modifier guarded() { require(true); _; }
    function effect() public guarded { revert(); }
    function boolean() external pure returns (bool) { return 2 > 1; }
    function unsigned() internal pure returns (uint8 result) { result = 7; }
    function signed() private pure returns (int256) { return -7; }
    function account() public view returns (address) { return msg.sender; }
    function payableAccount() public view returns (address payable) { return payable(msg.sender); }
    function tuple() public virtual returns (uint, bool);
    function dynamic() public pure returns (bytes memory) { return hex"1234"; }
    function multiple() public pure returns (uint, bool) { return (7, true); }
    function testSomething() public { revert(); }
    function invariantSomething() public { revert(); }
    function setUp() public { revert(); }
    function empty() public {}
    function emptyCommented() public { /* No work. */ }
    function implicitReturn() public pure returns (uint256 result) {}
    receive() external payable { revert(); }
    fallback() external { revert(); }
}
contract Other { function ignored() public { revert(); } }
"#;
        let mutants = function_body_mutants(source);
        let first = &mutants[0];
        let mut mutated = source.to_string();
        mutated.replace_range(
            first.span.lo().0 as usize..first.span.hi().0 as usize,
            &first.mutation.to_string(),
        );
        assert_eq!(
            mutated,
            source.replacen(
                "function effect() public guarded { revert(); }",
                "function effect() public guarded {}",
                1,
            )
        );
        assert_eq!(
            mutants.iter().map(|mutant| mutant.mutation.to_string()).collect::<Vec<_>>(),
            [
                "{}",
                "{ return true; }",
                "{ return false; }",
                "{ return 0; }",
                "{ return 1; }",
                "{ return 0; }",
                "{ return 1; }",
                "{ return -1; }",
                "{ return address(0); }",
                "{ return payable(address(0)); }",
                "{}",
                "{}",
                "{}",
                "{ return 1; }",
            ]
        );
    }

    #[test]
    fn function_body_replacements_skip_equivalent_constants() {
        for (ty, expression, expected) in [
            ("bool", "(true)", vec!["false"]),
            ("bool", "(/* Always on. */ true)", vec!["false"]),
            ("bool", "bool((false))", vec!["true"]),
            ("uint256", "0x0", vec!["1"]),
            ("uint8", "uint8(0)", vec!["1"]),
            ("uint256", "1_0e-1", vec!["0"]),
            ("int256", "-(int8((0x1)))", vec!["0", "1"]),
            ("int256", "int256(-1)", vec!["0", "1"]),
            ("int256", "-(-1)", vec!["0", "-1"]),
            ("int256", "-0", vec!["1", "-1"]),
            ("address", "address(uint160(0x0))", vec![]),
            ("address payable", "payable(address((0)))", vec![]),
            // Keep replacements when evaluating the original can have side effects.
            ("bool", "other()", vec!["true", "false"]),
            ("uint8", "uint8(other())", vec!["0", "1"]),
        ] {
            let source = format!(
                "contract Target {{ function target() public returns ({ty}) {{ return {expression}; }} }}"
            );
            let replacements = function_body_mutants(&source)
                .iter()
                .map(|mutant| mutant.mutation.to_string())
                .collect::<Vec<_>>();
            assert_eq!(
                replacements,
                expected.iter().map(|value| format!("{{ return {value}; }}")).collect::<Vec<_>>(),
                "{source}"
            );
        }
    }

    fn function_body_mutants(source: &str) -> Vec<Mutant> {
        let sess = Session::builder().with_silent_emitter(None).build();
        sess.enter(|| {
            let arena = Arena::new();
            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::Real("target.sol".into()),
                || Ok(source.to_string()),
            )
            .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor =
                MutantVisitor::with_operators("target.sol".into(), &[MutatorType::FunctionBody])
                    .with_source(source)
                    .with_contract_filter(|name| name == "Target");
            let _ = visitor.visit_source_unit(&ast);
            assert!(visitor.take_errors().is_empty());
            for mutant in &visitor.mutation_to_conduct {
                assert_eq!(
                    mutant.original,
                    &source[mutant.span.lo().0 as usize..mutant.span.hi().0 as usize]
                );
            }
            visitor.mutation_to_conduct
        })
    }
}
