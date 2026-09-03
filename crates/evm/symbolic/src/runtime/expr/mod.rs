use super::*;

mod bool;
mod cx;
pub(super) mod hashcons;
#[path = "expr.rs"]
mod word;

struct NoopModel;

impl SymbolicModelLookup for NoopModel {
    fn value(&self, _name: Symbol) -> Option<U256> {
        None
    }
}

pub(crate) use bool::*;
pub(crate) use cx::*;
pub(crate) use word::*;

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
                U256::from_be_bytes(keccak256(input).0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_evaluator_only_caches_conjunctions() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let one = SymExpr::constant(&mut cx, U256::from(1));
        let condition = SymBoolExpr::eq(&mut cx, value, one).not(&mut cx);
        let model = SymbolicModel::default();
        let mut evaluator = ModelEvaluator::new(&model);

        assert!(evaluator.eval_bool(&condition).unwrap());
        assert!(evaluator.bools.is_empty());
        let conjunction = SymBoolExpr::and(&mut cx, vec![condition.clone(), condition]);
        assert!(evaluator.eval_bool(&conjunction).unwrap());
        assert_eq!(evaluator.bools.len(), 1);
    }
}
