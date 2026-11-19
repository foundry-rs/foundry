use super::*;

mod bool;
mod cx;
pub(super) mod hashcons;
#[path = "expr.rs"]
mod word;

pub(crate) use bool::*;
pub(crate) use cx::*;
pub(crate) use word::*;

struct NoopModel;

impl SymbolicModelLookup for NoopModel {
    fn value(&self, _name: Symbol) -> Option<U256> {
        None
    }
}

/// Results from one deterministic bottom-up expression fold.
///
/// Keys borrow the original DAG so the memo table does not add strong references to source nodes.
#[derive(Default)]
struct ExpressionFoldCache<'a> {
    words: HashMap<&'a SymExpr, SymExpr>,
    bools: HashMap<&'a SymBoolExpr, SymBoolExpr>,
}

/// Structural digests that identify expressions in stable symbol names.
///
/// Each distinct hash-consed node is digested once. Formatting a node with `Debug` instead
/// prints the DAG as a tree, which grows exponentially when subexpressions are shared, as in a
/// chain of hashes over the previous hash.
#[derive(Default)]
pub(crate) struct ExpressionDigests<'a> {
    words: HashMap<&'a SymExpr, B256>,
    bools: HashMap<&'a SymBoolExpr, B256>,
}

impl<'a> ExpressionDigests<'a> {
    /// Returns one digest that identifies a sequence of expressions.
    pub(crate) fn identity(exprs: impl IntoIterator<Item = &'a SymExpr>) -> B256 {
        let mut digests = Self::default();
        let mut hasher = Keccak256::new();
        for expr in exprs {
            hasher.update(digests.digest(DigestNode::Word(expr)));
        }
        hasher.finalize()
    }

    /// Digests a node in post-order with an explicit stack, because expressions can be deeper
    /// than the thread stack allows for recursion.
    fn digest(&mut self, root: DigestNode<'a>) -> B256 {
        let mut stack = vec![(root, false)];
        while let Some((node, children_done)) = stack.pop() {
            if self.get(node).is_some() {
                continue;
            }
            if !children_done {
                stack.push((node, true));
                stack.extend(node.children().into_iter().map(|child| (child, false)));
                continue;
            }
            let mut hasher = Keccak256::new();
            match node {
                DigestNode::Word(expr) => {
                    match expr.kind() {
                        SymExprKind::Const(value) => {
                            hasher.update([0]);
                            hasher.update(value.to_be_bytes::<32>());
                        }
                        SymExprKind::Var(symbol) => {
                            hasher.update([1]);
                            hasher.update(symbol.id().get().to_be_bytes());
                        }
                        SymExprKind::GasLeft(symbol) => {
                            hasher.update([2]);
                            hasher.update(symbol.id().get().to_be_bytes());
                        }
                        // A hash name is already a digest of its preimage.
                        SymExprKind::Keccak { name, .. } => {
                            hasher.update([3]);
                            hasher.update(name.id().get().to_be_bytes());
                        }
                        SymExprKind::Hash { name, algorithm, .. } => {
                            hasher.update([4]);
                            hasher.update(algorithm.as_bytes());
                            hasher.update(name.id().get().to_be_bytes());
                        }
                        SymExprKind::Not(_) => hasher.update([5]),
                        SymExprKind::BinOp(op, _, _) => hasher.update([6, *op as u8]),
                        SymExprKind::TernOp(op, _, _, _) => hasher.update([7, *op as u8]),
                        SymExprKind::Ite(_, _, _) => hasher.update([8]),
                    }
                }
                DigestNode::Bool(expr) => match expr.kind() {
                    SymBoolExprKind::Const(value) => hasher.update([9, u8::from(*value)]),
                    SymBoolExprKind::Not(_) => hasher.update([10]),
                    SymBoolExprKind::And(_) => hasher.update([11]),
                    SymBoolExprKind::Cmp(op, _, _) => hasher.update([12, *op as u8]),
                },
            }
            for child in node.children() {
                hasher.update(self.get(child).expect("children are digested first"));
            }
            let digest = hasher.finalize();
            match node {
                DigestNode::Word(expr) => self.words.insert(expr, digest),
                DigestNode::Bool(expr) => self.bools.insert(expr, digest),
            };
        }
        self.get(root).expect("root is digested")
    }

    fn get(&self, node: DigestNode<'a>) -> Option<B256> {
        match node {
            DigestNode::Word(expr) => self.words.get(expr).copied(),
            DigestNode::Bool(expr) => self.bools.get(expr).copied(),
        }
    }
}

#[derive(Clone, Copy)]
enum DigestNode<'a> {
    Word(&'a SymExpr),
    Bool(&'a SymBoolExpr),
}

impl<'a> DigestNode<'a> {
    /// Returns the operands that contribute to the digest. Hash preimages are left out, because
    /// the hash name already commits to them.
    fn children(self) -> Vec<Self> {
        match self {
            Self::Word(expr) => match expr.kind() {
                SymExprKind::Const(_)
                | SymExprKind::Var(_)
                | SymExprKind::GasLeft(_)
                | SymExprKind::Keccak { .. }
                | SymExprKind::Hash { .. } => vec![],
                SymExprKind::Not(value) => vec![Self::Word(value)],
                SymExprKind::BinOp(_, left, right) => vec![Self::Word(left), Self::Word(right)],
                SymExprKind::TernOp(_, first, second, third) => {
                    vec![Self::Word(first), Self::Word(second), Self::Word(third)]
                }
                SymExprKind::Ite(condition, then, otherwise) => {
                    vec![Self::Bool(condition), Self::Word(then), Self::Word(otherwise)]
                }
            },
            Self::Bool(expr) => match expr.kind() {
                SymBoolExprKind::Const(_) => vec![],
                SymBoolExprKind::Not(value) => vec![Self::Bool(value)],
                SymBoolExprKind::And(values) => values.iter().map(Self::Bool).collect(),
                SymBoolExprKind::Cmp(_, left, right) => vec![Self::Word(left), Self::Word(right)],
            },
        }
    }
}

/// Evaluates hash-consed expressions once per model.
///
/// Symbolic expressions form a DAG, so recursively evaluating both operands without caching can
/// revisit the same node exponentially many times.
struct ModelEvaluator<'a, M: ?Sized> {
    model: &'a M,
    words: HashMap<SymExpr, U256>,
    bools: HashMap<SymBoolExpr, bool>,
}

impl<'a, M: SymbolicModelLookup + ?Sized> ModelEvaluator<'a, M> {
    fn new(model: &'a M) -> Self {
        Self { model, words: HashMap::default(), bools: HashMap::default() }
    }

    fn eval_word(&mut self, expr: &SymExpr) -> Result<U256, SymbolicError> {
        let kind = expr.kind();
        if let Some(var) = kind.get_eval_var() {
            return Ok(self.model.value(var).unwrap_or_default());
        }
        if let SymExprKind::Const(value) = kind {
            return Ok(*value);
        }
        if let Some(value) = self.words.get(expr) {
            return Ok(*value);
        }

        let value = match kind {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Hash { .. } => unreachable!("symbolic eval leaf handled above"),
            SymExprKind::Keccak { len, bytes, .. } => {
                let len = self.eval_word(len)?;
                let Ok(len) = usize::try_from(len) else {
                    return Err(SymbolicError::Solver(
                        "solver model uses an invalid keccak length".to_string(),
                    ));
                };
                if len > bytes.len() {
                    return Err(SymbolicError::Solver(
                        "solver model uses an invalid keccak length".to_string(),
                    ));
                }

                let mut input = Vec::with_capacity(len);
                for byte in bytes.iter().take(len) {
                    input.push((self.eval_word(byte)? & U256::from(0xff)).to::<u8>());
                }
                keccak256(input).into()
            }
            SymExprKind::Not(value) => !self.eval_word(value)?,
            SymExprKind::BinOp(op, left, right) => {
                op.eval(self.eval_word(left)?, self.eval_word(right)?)
            }
            SymExprKind::TernOp(op, left, right, modulus) => {
                op.eval(self.eval_word(left)?, self.eval_word(right)?, self.eval_word(modulus)?)
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                if self.eval_bool(condition)? {
                    self.eval_word(then_expr)?
                } else {
                    self.eval_word(else_expr)?
                }
            }
        };
        self.words.insert(expr.clone(), value);
        Ok(value)
    }

    fn eval_bool(&mut self, expr: &SymBoolExpr) -> Result<bool, SymbolicError> {
        let kind = expr.kind();
        if let SymBoolExprKind::Const(value) = kind {
            return Ok(*value);
        }
        // `Not` and `Cmp` cheaply recombine child results, so memoizing them adds one-use entries
        // for ordinary path constraints. Conjunctions can share additional Boolean work.
        let cache_result = matches!(kind, SymBoolExprKind::And(_));
        if cache_result && let Some(value) = self.bools.get(expr) {
            return Ok(*value);
        }

        let value = match kind {
            SymBoolExprKind::Const(_) => unreachable!("symbolic eval leaf handled above"),
            SymBoolExprKind::Not(value) => !self.eval_bool(value)?,
            SymBoolExprKind::And(values) => {
                let mut result = true;
                for value in values.iter() {
                    if !self.eval_bool(value)? {
                        result = false;
                        break;
                    }
                }
                result
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                op.eval(self.eval_word(left)?, self.eval_word(right)?)
            }
        };
        if cache_result {
            self.bools.insert(expr.clone(), value);
        }
        Ok(value)
    }
}

pub(crate) fn eval_model_constraints<M: SymbolicModelLookup + ?Sized>(
    constraints: &[SymBoolExpr],
    model: &M,
) -> bool {
    let mut evaluator = ModelEvaluator::new(model);
    constraints.iter().all(|constraint| evaluator.eval_bool(constraint).unwrap_or(false))
}
