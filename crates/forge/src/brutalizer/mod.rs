use std::{ops::ControlFlow, path::Path};

use foundry_config::Config;
use solar::{
    ast::{
        Block, CallArgsKind, ElementaryType, Expr, ExprKind, FunctionKind, ItemFunction, Span,
        StmtKind, TypeKind, TypeSize, Visibility,
        interface::{Session, source_map::FileName},
        visit::Visit,
    },
    interface::BytePos,
    parse::Parser,
};

mod utils;
use utils::{extract_span_text, is_eligible_function, span_seed, splitmix64};

struct BrutalizerTransform {
    span: Span,
    replacement: String,
    is_insertion: bool,
}

struct BrutalizerVisitor<'src> {
    transforms: Vec<BrutalizerTransform>,
    source: &'src str,
    current_fn_visibility: Option<Visibility>,
    current_fn_kind: Option<FunctionKind>,
    current_fn_has_assembly: bool,
}

impl<'src> BrutalizerVisitor<'src> {
    fn new(source: &'src str) -> Self {
        Self {
            transforms: Vec::new(),
            source,
            current_fn_visibility: None,
            current_fn_kind: None,
            current_fn_has_assembly: false,
        }
    }
}

impl<'ast> Visit<'ast> for BrutalizerVisitor<'ast> {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        let (callee, call_args) = match &expr.kind {
            ExprKind::Call(callee, args) => (callee, args),
            _ => return self.walk_expr(expr),
        };

        let ty = match &callee.kind {
            ExprKind::Type(ty) => ty,
            _ => return self.walk_expr(expr),
        };

        let args_exprs = match &call_args.kind {
            CallArgsKind::Unnamed(exprs) => exprs,
            _ => return self.walk_expr(expr),
        };

        if args_exprs.is_empty() {
            return self.walk_expr(expr);
        }

        let arg_text = extract_span_text(self.source, args_exprs[0].span);
        if arg_text.is_empty() {
            return self.walk_expr(expr);
        }

        let mask = deterministic_mask(expr.span);

        if let Some(brutalized) = brutalize_by_type(ty, &arg_text, &mask) {
            self.transforms.push(BrutalizerTransform {
                span: expr.span,
                replacement: brutalized,
                is_insertion: false,
            });
        }

        self.walk_expr(expr)
    }

    fn visit_item_function(
        &mut self,
        func: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if let Some(ref body) = func.body {
            let visibility = func.header.visibility();
            let kind = Some(func.kind);
            let has_assembly = block_contains_assembly(body);

            self.current_fn_visibility = visibility;
            self.current_fn_kind = kind;
            self.current_fn_has_assembly = has_assembly;

            if has_assembly && is_eligible_function(visibility, kind) {
                let insert_pos = body.span.lo().0 + 1;
                let insert_span = Span::new(BytePos(insert_pos), BytePos(insert_pos));

                let memory_asm = generate_memory_brutalization_assembly(insert_span);
                self.transforms.push(BrutalizerTransform {
                    span: insert_span,
                    replacement: memory_asm,
                    is_insertion: true,
                });

                let fmp_asm = generate_fmp_misalignment_assembly(insert_span);
                self.transforms.push(BrutalizerTransform {
                    span: insert_span,
                    replacement: fmp_asm,
                    is_insertion: true,
                });
            }
        }

        let result = self.walk_item_function(func);

        self.current_fn_visibility = None;
        self.current_fn_kind = None;
        self.current_fn_has_assembly = false;

        result
    }
}

pub fn brutalize_source(path: &Path, source: &str) -> eyre::Result<String> {
    let sess = Session::builder().with_silent_emitter(None).build();

    let result = sess.enter(|| -> solar::interface::Result<Vec<BrutalizerTransform>> {
        let arena = solar::ast::Arena::new();
        let mut parser = Parser::from_lazy_source_code(
            &sess,
            &arena,
            FileName::from(path.to_path_buf()),
            || Ok(source.to_string()),
        )?;

        let ast = parser.parse_file().map_err(|e| e.emit())?;

        let mut visitor = BrutalizerVisitor::new(source);
        let _ = visitor.visit_source_unit(&ast);

        Ok(visitor.transforms)
    });

    let mut transforms = match result {
        Ok(t) => t,
        Err(_) => eyre::bail!("failed to parse {}", path.display()),
    };

    transforms.sort_by(|a, b| {
        let a_pos = a.span.lo().0;
        let b_pos = b.span.lo().0;
        b_pos.cmp(&a_pos).then_with(|| {
            let a_hi = a.span.hi().0;
            let b_hi = b.span.hi().0;
            b_hi.cmp(&a_hi)
        })
    });

    let mut result = source.to_string();
    for transform in &transforms {
        let lo = transform.span.lo().0 as usize;
        let hi = transform.span.hi().0 as usize;

        if transform.is_insertion {
            result.insert_str(lo, &transform.replacement);
        } else {
            result.replace_range(lo..hi, &transform.replacement);
        }
    }

    Ok(result)
}

/// Brutalize all .sol source files in a temp project directory.
///
/// Walks the src directory under `temp_dir`, parses each .sol file, applies all
/// brutalizations (value XOR, memory, FMP), and writes the result back in-place.
///
/// Returns the number of files brutalized.
pub fn brutalize_project(config: &Config, temp_dir: &Path) -> eyre::Result<usize> {
    let src_rel = config.src.strip_prefix(&config.root).unwrap_or(&config.src);
    let src_dir = temp_dir.join(src_rel);

    if !src_dir.exists() {
        return Ok(0);
    }

    brutalize_sol_files_in_dir(&src_dir)
}

fn brutalize_sol_files_in_dir(dir: &Path) -> eyre::Result<usize> {
    let mut count = 0;
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            count += brutalize_sol_files_in_dir(&path)?;
        } else if path.extension().is_some_and(|ext| ext == "sol") {
            let source = std::fs::read_to_string(&path)?;
            let brutalized = brutalize_source(&path, &source)?;
            if brutalized != source {
                std::fs::write(&path, brutalized)?;
                count += 1;
            }
        }
    }
    Ok(count)
}

fn deterministic_mask(span: Span) -> String {
    let h = span_seed(span);
    let mask = if h == 0 { 1 } else { h };
    format!("0x{mask:016x}")
}

fn brutalize_by_type(ty: &solar::ast::Type<'_>, arg_text: &str, mask: &str) -> Option<String> {
    match &ty.kind {
        TypeKind::Elementary(elem_ty) => match elem_ty {
            ElementaryType::Address(_) => Some(brutalize_address(arg_text, mask)),
            ElementaryType::UInt(size) => brutalize_uint(*size, arg_text, mask),
            ElementaryType::Int(size) => brutalize_int(*size, arg_text, mask),
            ElementaryType::FixedBytes(size) => brutalize_fixed_bytes(*size, arg_text, mask),
            ElementaryType::Bool => None,
            ElementaryType::Bytes | ElementaryType::String => None,
            ElementaryType::Fixed(..) | ElementaryType::UFixed(..) => None,
        },
        _ => None,
    }
}

fn brutalize_address(arg_text: &str, mask: &str) -> String {
    format!("address(uint160(uint256(uint160({arg_text})) ^ uint256({mask})))")
}

fn brutalize_uint(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    Some(format!("uint{actual_bits}(uint256({arg_text}) ^ uint256({mask}))"))
}

fn brutalize_int(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    Some(format!("int{actual_bits}(int256({arg_text}) ^ int256(uint256({mask})))"))
}

fn brutalize_fixed_bytes(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bytes = size.bytes_raw();
    if bytes >= 32 || bytes == 0 {
        return None;
    }
    Some(format!("bytes{bytes}(bytes32(uint256(bytes32({arg_text})) ^ uint256({mask})))"))
}

fn generate_memory_brutalization_assembly(span: Span) -> String {
    let s = span_seed(span);
    let w0 = splitmix64(s);
    let w1 = splitmix64(s.wrapping_add(1));
    let w2 = splitmix64(s.wrapping_add(2));
    let w3 = splitmix64(s.wrapping_add(3));
    let s0 = splitmix64(s.wrapping_add(4));
    let s1 = splitmix64(s.wrapping_add(5));
    let s2 = splitmix64(s.wrapping_add(6));
    let s3 = splitmix64(s.wrapping_add(7));
    format!(
        " assembly {{ \
        mstore(0x00, 0x{w0:016x}{w1:016x}) \
        mstore(0x20, 0x{w2:016x}{w3:016x}) \
        let _b_p := mload(0x40) \
        mstore(_b_p, 0x{s0:016x}{s1:016x}{s2:016x}{s3:016x}) \
        for {{ let _b_i := 0x20 }} lt(_b_i, 0x400) {{ _b_i := add(_b_i, 0x20) }} {{ \
        mstore(add(_b_p, _b_i), keccak256(add(_b_p, sub(_b_i, 0x20)), 0x20)) \
        }} \
        }} "
    )
}

fn generate_fmp_misalignment_assembly(span: Span) -> String {
    let offset = deterministic_fmp_offset(span);
    format!(" assembly {{ mstore(0x40, add(mload(0x40), {offset})) }} ")
}

fn deterministic_fmp_offset(span: Span) -> u8 {
    ((span_seed(span) % 31) as u8) | 1
}

fn block_contains_assembly(block: &Block<'_>) -> bool {
    block.stmts.iter().any(|stmt| stmt_contains_assembly(&stmt.kind))
}

fn stmt_contains_assembly(kind: &StmtKind<'_>) -> bool {
    match kind {
        StmtKind::Assembly(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => block_contains_assembly(block),
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_contains_assembly(&then_stmt.kind)
                || else_stmt.as_ref().is_some_and(|s| stmt_contains_assembly(&s.kind))
        }
        StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => stmt_contains_assembly(&body.kind),
        StmtKind::For { body, .. } => stmt_contains_assembly(&body.kind),
        StmtKind::Try(try_stmt) => {
            try_stmt.clauses.iter().any(|clause| block_contains_assembly(&clause.block))
        }
        _ => false,
    }
}
