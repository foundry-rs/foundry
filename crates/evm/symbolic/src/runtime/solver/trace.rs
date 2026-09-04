use super::*;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::BufWriter,
    sync::{OnceLock, atomic::AtomicU64},
};

const TRACE_DIRECTORY_ENV: &str = "FOUNDRY_SYMBOLIC_QUERY_TRACE_DIR";
const TRACE_SCHEMA: &str = "foundry:symbolic-query-dag@v1";
const TRACE_SCHEMA_VERSION: u32 = 1;

static TRACE_DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
static TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
struct QueryTrace {
    schema: &'static str,
    schema_version: u32,
    stage: TraceStage,
    request: QueryRequest,
    variable_count: usize,
    words: Vec<WordNode>,
    predicates: Vec<PredicateNode>,
    assertions: Vec<u32>,
    baseline: Option<Baseline>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TraceStage {
    Normalized,
    Native,
    Backend,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum QueryRequest {
    Check,
    Model,
}

#[derive(Serialize)]
struct Baseline {
    outcome: &'static str,
    wall_time_ns: u64,
    smt_input_bytes: u64,
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WordNode {
    Constant { value: String },
    Variable { variable: u32 },
    Opaque { variable: u32, kind: &'static str },
    Not { value: u32 },
    Binary { operator: &'static str, left: u32, right: u32 },
    Ternary { operator: &'static str, left: u32, right: u32, modulus: u32 },
    Ite { condition: u32, then_value: u32, else_value: u32 },
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum PredicateNode {
    Constant { value: bool },
    Not { value: u32 },
    And { values: Vec<u32> },
    Compare { operator: &'static str, left: u32, right: u32 },
}

struct TraceBuilder<'a> {
    words: Vec<WordNode>,
    predicates: Vec<PredicateNode>,
    word_ids: HashMap<&'a SymExpr, u32>,
    predicate_ids: HashMap<&'a SymBoolExpr, u32>,
    variable_ids: HashMap<Symbol, u32>,
}

impl<'a> TraceBuilder<'a> {
    fn new() -> Self {
        Self {
            words: Vec::new(),
            predicates: Vec::new(),
            word_ids: HashMap::default(),
            predicate_ids: HashMap::default(),
            variable_ids: HashMap::default(),
        }
    }

    fn finish(mut self, constraints: &'a [SymBoolExpr]) -> QueryTraceDag {
        let assertions = constraints.iter().map(|constraint| self.predicate(constraint)).collect();
        QueryTraceDag {
            variable_count: self.variable_ids.len(),
            words: self.words,
            predicates: self.predicates,
            assertions,
        }
    }

    fn variable(&mut self, symbol: Symbol) -> u32 {
        let next = self.variable_ids.len().try_into().expect("symbolic trace variable overflow");
        *self.variable_ids.entry(symbol).or_insert(next)
    }

    fn word(&mut self, expression: &'a SymExpr) -> u32 {
        if let Some(id) = self.word_ids.get(expression) {
            return *id;
        }

        let node = match expression.kind() {
            SymExprKind::Const(value) => WordNode::Constant { value: format!("{value:#066x}") },
            SymExprKind::Var(symbol) => WordNode::Variable { variable: self.variable(*symbol) },
            SymExprKind::GasLeft(symbol) => {
                WordNode::Opaque { variable: self.variable(*symbol), kind: "gas_left" }
            }
            SymExprKind::Keccak { name, .. } => {
                WordNode::Opaque { variable: self.variable(*name), kind: "keccak" }
            }
            SymExprKind::Hash { name, .. } => {
                WordNode::Opaque { variable: self.variable(*name), kind: "hash" }
            }
            SymExprKind::Not(value) => WordNode::Not { value: self.word(value) },
            SymExprKind::BinOp(operator, left, right) => WordNode::Binary {
                operator: binary_operator(*operator),
                left: self.word(left),
                right: self.word(right),
            },
            SymExprKind::TernOp(operator, left, right, modulus) => WordNode::Ternary {
                operator: ternary_operator(*operator),
                left: self.word(left),
                right: self.word(right),
                modulus: self.word(modulus),
            },
            SymExprKind::Ite(condition, then_value, else_value) => WordNode::Ite {
                condition: self.predicate(condition),
                then_value: self.word(then_value),
                else_value: self.word(else_value),
            },
        };
        let id = self.words.len().try_into().expect("symbolic trace word overflow");
        self.words.push(node);
        self.word_ids.insert(expression, id);
        id
    }

    fn predicate(&mut self, expression: &'a SymBoolExpr) -> u32 {
        if let Some(id) = self.predicate_ids.get(expression) {
            return *id;
        }

        let node = match expression.kind() {
            SymBoolExprKind::Const(value) => PredicateNode::Constant { value: *value },
            SymBoolExprKind::Not(value) => PredicateNode::Not { value: self.predicate(value) },
            SymBoolExprKind::And(values) => PredicateNode::And {
                values: values.iter().map(|value| self.predicate(value)).collect(),
            },
            SymBoolExprKind::Cmp(operator, left, right) => PredicateNode::Compare {
                operator: comparison_operator(*operator),
                left: self.word(left),
                right: self.word(right),
            },
        };
        let id = self.predicates.len().try_into().expect("symbolic trace predicate overflow");
        self.predicates.push(node);
        self.predicate_ids.insert(expression, id);
        id
    }
}

struct QueryTraceDag {
    variable_count: usize,
    words: Vec<WordNode>,
    predicates: Vec<PredicateNode>,
    assertions: Vec<u32>,
}

pub(super) struct PendingQueryTrace {
    directory: PathBuf,
    occurrence: Option<TraceOccurrence>,
    request: QueryRequest,
    dag: QueryTraceDag,
    smt_input_bytes: u64,
}

pub(super) struct EncodedQueryTrace {
    pub(super) occurrence: TraceOccurrence,
    pub(super) bytes: Vec<u8>,
    pub(super) outcome: &'static str,
}

pub(super) struct TraceOccurrence {
    stem: String,
}

impl TraceOccurrence {
    pub(super) fn stem(&self) -> &str {
        &self.stem
    }

    #[cfg(test)]
    pub(super) fn for_test(stem: &str) -> Self {
        Self { stem: stem.to_owned() }
    }
}

pub(super) fn write_normalized_query_trace(
    constraints: &[SymBoolExpr],
    model: bool,
) -> Result<(), SymbolicError> {
    write_query_trace(constraints, model, TraceStage::Normalized)
}

pub(super) fn write_native_query_trace(
    constraints: &[SymBoolExpr],
    model: bool,
) -> Result<(), SymbolicError> {
    write_query_trace(constraints, model, TraceStage::Native)
}

fn write_query_trace(
    constraints: &[SymBoolExpr],
    model: bool,
    stage: TraceStage,
) -> Result<(), SymbolicError> {
    let Some(directory) = trace_directory() else { return Ok(()) };
    if super::fresh_z3_capture::capture_directory_is_configured() {
        return Err(SymbolicError::Solver(
            "FOUNDRY_SYMBOLIC_QUERY_TRACE_DIR cannot be combined with fresh Z3 capture".to_string(),
        ));
    }
    let dag = TraceBuilder::new().finish(constraints);
    let trace = QueryTrace {
        schema: TRACE_SCHEMA,
        schema_version: TRACE_SCHEMA_VERSION,
        stage,
        request: if model { QueryRequest::Model } else { QueryRequest::Check },
        variable_count: dag.variable_count,
        words: dag.words,
        predicates: dag.predicates,
        assertions: dag.assertions,
        baseline: None,
    };
    write_trace_file(directory, &trace)
}

pub(super) fn capture_query_trace(
    constraints: &[SymBoolExpr],
    model: bool,
    smt_input_bytes: u64,
    directory_override: Option<&Path>,
) -> Option<PendingQueryTrace> {
    let occurrence =
        directory_override.is_some().then(|| next_trace_occurrence(TraceStage::Backend));
    let directory = directory_override
        .map(Path::to_path_buf)
        .or_else(|| trace_directory().map(Path::to_path_buf))?;
    Some(PendingQueryTrace {
        directory,
        occurrence,
        request: if model { QueryRequest::Model } else { QueryRequest::Check },
        dag: TraceBuilder::new().finish(constraints),
        smt_input_bytes,
    })
}

impl PendingQueryTrace {
    pub(super) const fn occurrence(&self) -> Option<&TraceOccurrence> {
        self.occurrence.as_ref()
    }

    pub(super) fn write(
        mut self,
        result: &Result<String, SymbolicError>,
        wall_time: Duration,
    ) -> Result<(), SymbolicError> {
        let directory = self.directory.clone();
        let occurrence =
            self.occurrence.take().unwrap_or_else(|| next_trace_occurrence(TraceStage::Backend));
        let trace = self.encode_for_occurrence(result, wall_time, occurrence)?;
        write_trace_bytes(&directory, &trace.occurrence, &trace.bytes)
    }

    pub(super) fn encode(
        mut self,
        result: &Result<String, SymbolicError>,
        wall_time: Duration,
    ) -> Result<EncodedQueryTrace, SymbolicError> {
        let occurrence = self.occurrence.take().ok_or_else(|| {
            SymbolicError::Solver(
                "fresh Z3 query trace is missing its reserved occurrence".to_string(),
            )
        })?;
        self.encode_for_occurrence(result, wall_time, occurrence)
    }

    fn encode_for_occurrence(
        self,
        result: &Result<String, SymbolicError>,
        wall_time: Duration,
        occurrence: TraceOccurrence,
    ) -> Result<EncodedQueryTrace, SymbolicError> {
        let outcome = solver_outcome(result);
        let trace = QueryTrace {
            schema: TRACE_SCHEMA,
            schema_version: TRACE_SCHEMA_VERSION,
            stage: TraceStage::Backend,
            request: self.request,
            variable_count: self.dag.variable_count,
            words: self.dag.words,
            predicates: self.dag.predicates,
            assertions: self.dag.assertions,
            baseline: Some(Baseline {
                outcome,
                wall_time_ns: wall_time.as_nanos().try_into().unwrap_or(u64::MAX),
                smt_input_bytes: self.smt_input_bytes,
            }),
        };
        let mut bytes = serde_json::to_vec(&trace).map_err(|error| {
            SymbolicError::Solver(format!("failed to encode query trace: {error}"))
        })?;
        bytes.push(b'\n');
        Ok(EncodedQueryTrace { occurrence, bytes, outcome })
    }
}

fn trace_directory() -> Option<&'static Path> {
    TRACE_DIRECTORY
        .get_or_init(|| {
            std::env::var_os(TRACE_DIRECTORY_ENV).filter(|path| !path.is_empty()).map(PathBuf::from)
        })
        .as_deref()
}

pub(super) fn configured_trace_directory() -> Option<&'static Path> {
    trace_directory()
}

fn write_trace_file(directory: &Path, trace: &QueryTrace) -> Result<(), SymbolicError> {
    let occurrence = next_trace_occurrence(trace.stage);
    let mut bytes = serde_json::to_vec(trace).map_err(|error| {
        SymbolicError::Solver(format!("failed to encode symbolic query trace: {error}"))
    })?;
    bytes.push(b'\n');
    write_trace_bytes(directory, &occurrence, &bytes)
}

fn next_trace_occurrence(stage: TraceStage) -> TraceOccurrence {
    let sequence = TRACE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stage = match stage {
        TraceStage::Normalized => "normalized",
        TraceStage::Native => "native",
        TraceStage::Backend => "backend",
    };
    TraceOccurrence { stem: format!("{stage}-{}-{sequence:08}", std::process::id()) }
}

fn write_trace_bytes(
    directory: &Path,
    occurrence: &TraceOccurrence,
    bytes: &[u8],
) -> Result<(), SymbolicError> {
    fs::create_dir_all(directory).map_err(|error| trace_error(directory, error))?;
    let path = directory.join(format!("{}.json", occurrence.stem));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| trace_error(&path, error))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(bytes).map_err(|error| trace_error(&path, error))?;
    writer.flush().map_err(|error| trace_error(&path, error))
}

fn trace_error(path: &Path, error: std::io::Error) -> SymbolicError {
    SymbolicError::Solver(format!(
        "failed to write symbolic query trace {}: {error}",
        path.display()
    ))
}

fn solver_outcome(result: &Result<String, SymbolicError>) -> &'static str {
    match result {
        Ok(output) => match output.lines().next().unwrap_or_default().trim() {
            "sat" => "sat",
            "unsat" => "unsat",
            "unknown" => "unknown",
            _ => "unexpected",
        },
        Err(SymbolicError::SolverUnknown) => "unknown",
        Err(_) => "error",
    }
}

const fn binary_operator(operator: SymBinOp) -> &'static str {
    match operator {
        SymBinOp::Add => "add",
        SymBinOp::Sub => "sub",
        SymBinOp::Mul => "mul",
        SymBinOp::UDiv => "udiv",
        SymBinOp::URem => "urem",
        SymBinOp::SDiv => "sdiv",
        SymBinOp::SRem => "srem",
        SymBinOp::And => "and",
        SymBinOp::Or => "or",
        SymBinOp::Xor => "xor",
        SymBinOp::Shl => "shl",
        SymBinOp::Shr => "shr",
        SymBinOp::Sar => "sar",
    }
}

const fn ternary_operator(operator: SymTernOp) -> &'static str {
    match operator {
        SymTernOp::AddMod => "add_mod",
        SymTernOp::MulMod => "mul_mod",
    }
}

const fn comparison_operator(operator: SymCmpOp) -> &'static str {
    match operator {
        SymCmpOp::Eq => "eq",
        SymCmpOp::Ult => "ult",
        SymCmpOp::Ugt => "ugt",
        SymCmpOp::Ule => "ule",
        SymCmpOp::Uge => "uge",
        SymCmpOp::Slt => "slt",
        SymCmpOp::Sgt => "sgt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_dag_uses_numeric_variables_and_shared_nodes() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "private_name");
        let one = SymExpr::constant(&mut cx, U256::from(1));
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, value, one.clone());
        let first = SymBoolExpr::eq(&mut cx, sum.clone(), one.clone());
        let second = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, sum, one);
        let constraints = [first, second];

        let dag = TraceBuilder::new().finish(&constraints);
        let encoded = serde_json::to_string(&QueryTrace {
            schema: TRACE_SCHEMA,
            schema_version: TRACE_SCHEMA_VERSION,
            stage: TraceStage::Backend,
            request: QueryRequest::Check,
            variable_count: dag.variable_count,
            words: dag.words,
            predicates: dag.predicates,
            assertions: dag.assertions,
            baseline: Some(Baseline { outcome: "sat", wall_time_ns: 1, smt_input_bytes: 2 }),
        })
        .unwrap();

        assert_eq!(encoded.matches("\"operator\":\"add\"").count(), 1);
        assert!(!encoded.contains("private_name"));
        assert!(encoded.contains("\"variable_count\":1"));
    }
}
