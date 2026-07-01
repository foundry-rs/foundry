use super::{runtime::*, *};

#[derive(Clone, Debug)]
pub(super) struct SymbolicCalldata {
    bytes: SymBytes,
    inputs: Vec<SymbolicInput>,
    constraints: Vec<SymBoolExpr>,
}

impl SymbolicCalldata {
    pub(super) fn variants(
        function: &Function,
        config: &SymbolicConfig,
        cx: &mut SymCx,
    ) -> Result<Vec<Self>, SymbolicError> {
        Self::variants_with_prefix(function, config, cx, "calldata")
    }

    pub(super) fn selector_only(
        cx: &mut SymCx,
        function: &Function,
    ) -> Result<Self, SymbolicError> {
        if !function.inputs.is_empty() {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic invariant `{}` must take no parameters",
                function.name
            )));
        }
        Ok(Self {
            bytes: SymBytes::concrete(cx, function.selector().to_vec()),
            inputs: Vec::new(),
            constraints: Vec::new(),
        })
    }

    pub(super) fn variants_with_prefix(
        function: &Function,
        config: &SymbolicConfig,
        cx: &mut SymCx,
        prefix: &str,
    ) -> Result<Vec<Self>, SymbolicError> {
        let variant_limit = calldata_variant_limit(config);
        let mut builder = SymbolicAbiBuilder::new(config, cx);
        let mut variants = vec![(SymbolicAbiState::default(), Vec::new())];
        for (idx, input) in function.inputs.iter().enumerate() {
            let ty = input.selector_type();
            let mut next_variants = Vec::new();
            for (state, inputs) in variants {
                for (state, input) in SymbolicInput::variants(
                    &mut builder,
                    state,
                    prefix,
                    idx,
                    Some(input.name.as_str()),
                    ty.as_ref(),
                )? {
                    let mut inputs = inputs.clone();
                    inputs.push(input);
                    push_variant(&mut next_variants, (state, inputs), variant_limit)?;
                }
            }
            variants = next_variants;
        }

        validate_positional_dynamic_lengths(
            config,
            variants.iter().map(|(state, _)| state.positional_dynamic_index).max().unwrap_or(0),
        )?;

        let mut out = Vec::with_capacity(variants.len());
        for (state, inputs) in variants {
            let selector = SymBytes::concrete(builder.cx, function.selector().to_vec());
            let encoded = builder.encode_sequence(inputs.iter().map(|input| &input.value));
            let bytes = SymBytes::concat(builder.cx, [selector, encoded]);
            if bytes.len() > config.max_calldata_bytes as usize {
                return Err(SymbolicError::Unsupported(
                    "symbolic calldata size exceeds configured max",
                ));
            }

            out.push(Self { bytes, inputs, constraints: state.constraints });
        }
        Ok(out)
    }

    pub(super) fn call_data(&self, cx: &mut SymCx) -> SymCalldata {
        SymCalldata::from_bytes(cx, self.bytes.clone())
    }

    /// Returns symbolic calldata constraints.
    pub(super) fn constraints(&self) -> &[SymBoolExpr] {
        &self.constraints
    }

    /// Consumes this symbolic calldata into its constraints.
    pub(super) fn into_constraints(self) -> Vec<SymBoolExpr> {
        self.constraints
    }

    pub(super) fn model_to_args(
        &self,
        cx: &mut SymCx,
        model: &(impl SymbolicModelLookup + ?Sized),
    ) -> Result<Vec<DynSolValue>, SymbolicError> {
        self.inputs.iter().map(|input| input.value.model_value(cx, model)).collect()
    }

    pub(super) fn seed_model(
        &self,
        cx: &mut SymCx,
        seed: &SymbolicConcreteInput,
    ) -> Option<SymbolicModel> {
        if seed.args.len() != self.inputs.len() {
            return None;
        }

        let mut model = SymbolicModel::default();
        for (input, arg) in self.inputs.iter().zip(&seed.args) {
            if !input.value.seed_model_value(cx, &mut model, arg) {
                return None;
            }
        }

        for constraint in &self.constraints {
            if constraint.eval_model_if_complete(&model).ok().flatten() != Some(true) {
                return None;
            }
        }

        let calldata = self.bytes.eval_model(cx, &model).ok()?;
        (calldata.as_slice() == seed.calldata.as_ref()).then_some(model)
    }
}

#[derive(Clone, Debug)]
pub(super) struct SymbolicInput {
    value: SymbolicAbiValue,
}

impl SymbolicInput {
    pub(super) fn variants<'a, 'cx>(
        builder: &mut SymbolicAbiBuilder<'a, 'cx>,
        state: SymbolicAbiState,
        prefix: &str,
        idx: usize,
        abi_name: Option<&str>,
        ty: &str,
    ) -> Result<Vec<(SymbolicAbiState, Self)>, SymbolicError> {
        let ty =
            DynSolType::parse(ty).map_err(|_| SymbolicError::UnsupportedAbi(ty.to_string()))?;
        let name = format!("{prefix}_{idx}");
        let aliases =
            abi_name.filter(|name| !name.is_empty()).map(str::to_string).into_iter().collect();
        builder.value_variants(state, name, aliases, &ty).map(|variants| {
            variants.into_iter().map(|(state, value)| (state, Self { value })).collect()
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct SymbolicAbiState {
    constraints: Vec<SymBoolExpr>,
    positional_dynamic_index: usize,
}

#[derive(Debug)]
pub(super) struct SymbolicAbiBuilder<'a, 'cx> {
    config: &'a SymbolicConfig,
    cx: &'cx mut SymCx,
}

struct SymbolicAbiEncoder<'cx> {
    cx: &'cx mut SymCx,
}

impl<'a, 'cx> SymbolicAbiBuilder<'a, 'cx> {
    /// Constructs a new instance.
    pub(super) const fn new(config: &'a SymbolicConfig, cx: &'cx mut SymCx) -> Self {
        Self { config, cx }
    }

    pub(super) fn value(
        &mut self,
        state: &mut SymbolicAbiState,
        name: String,
        aliases: Vec<String>,
        ty: &DynSolType,
    ) -> Result<SymbolicAbiValue, SymbolicError> {
        Ok(match ty {
            DynSolType::Bool => {
                let word = self.fresh_word(&name);
                state.constraints.push(self.cx.cmp_word_const(
                    SymBoolExprOp::Ult,
                    &word,
                    U256::from(2),
                ));
                SymbolicAbiValue::Bool { word }
            }
            DynSolType::Uint(bits) => {
                let word = self.fresh_word(&name);
                self.constrain_uint(state, &word, *bits);
                SymbolicAbiValue::Uint { bits: *bits, word }
            }
            DynSolType::Int(bits) => {
                let word = self.fresh_word(&name);
                self.constrain_int(state, &word, *bits);
                SymbolicAbiValue::Int { bits: *bits, word }
            }
            DynSolType::FixedBytes(size) => {
                let bytes = (0..*size)
                    .map(|idx| self.fresh_byte(state, &format!("{name}_{idx}"), false))
                    .collect();
                SymbolicAbiValue::FixedBytes { bytes: SymBytes::exprs(self.cx, bytes), size: *size }
            }
            DynSolType::Address => {
                let word = self.fresh_word(&name);
                self.constrain_uint(state, &word, 160);
                SymbolicAbiValue::Address { word }
            }
            DynSolType::Function => {
                return Err(SymbolicError::UnsupportedAbi("function".to_string()));
            }
            DynSolType::Bytes => {
                let len = self.next_dynamic_length(state, &name, &aliases, DynamicKind::Bytes)?;
                let bytes = (0..len)
                    .map(|idx| self.fresh_byte(state, &format!("{name}_{idx}"), false))
                    .collect();
                SymbolicAbiValue::Bytes {
                    len: self.cx.constant(U256::from(len)),
                    bytes: SymBytes::exprs(self.cx, bytes),
                }
            }
            DynSolType::String => {
                let len = self.next_dynamic_length(state, &name, &aliases, DynamicKind::String)?;
                let bytes = (0..len)
                    .map(|idx| self.fresh_byte(state, &format!("{name}_{idx}"), true))
                    .collect();
                SymbolicAbiValue::String { bytes: SymBytes::exprs(self.cx, bytes) }
            }
            DynSolType::Array(inner) => {
                let len = self.next_dynamic_length(state, &name, &aliases, DynamicKind::Array)?;
                SymbolicAbiValue::Array {
                    elements: (0..len)
                        .map(|idx| {
                            self.value(
                                state,
                                format!("{name}_{idx}"),
                                child_aliases(&aliases, idx),
                                inner,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            DynSolType::FixedArray(inner, len) => SymbolicAbiValue::FixedArray {
                elements: (0..*len)
                    .map(|idx| {
                        self.value(
                            state,
                            format!("{name}_{idx}"),
                            child_aliases(&aliases, idx),
                            inner,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::Tuple(types) => SymbolicAbiValue::Tuple {
                elements: types
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| {
                        self.value(state, format!("{name}_{idx}"), child_aliases(&aliases, idx), ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::CustomStruct { tuple, .. } => SymbolicAbiValue::Tuple {
                elements: tuple
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| {
                        self.value(state, format!("{name}_{idx}"), child_aliases(&aliases, idx), ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })
    }

    pub(super) fn value_variants(
        &mut self,
        state: SymbolicAbiState,
        name: String,
        aliases: Vec<String>,
        ty: &DynSolType,
    ) -> Result<Vec<(SymbolicAbiState, SymbolicAbiValue)>, SymbolicError> {
        Ok(match ty {
            DynSolType::Bytes => {
                let mut state = state;
                let lengths = self.next_dynamic_length_options(
                    &mut state,
                    &name,
                    &aliases,
                    DynamicKind::Bytes,
                )?;
                let limit = calldata_variant_limit(self.config);
                let mut variants = Vec::new();
                for len in lengths {
                    let mut state = state.clone();
                    let bytes = (0..len as usize)
                        .map(|idx| self.fresh_byte(&mut state, &format!("{name}_{idx}"), false))
                        .collect();
                    let value = SymbolicAbiValue::Bytes {
                        len: self.cx.constant(U256::from(len)),
                        bytes: SymBytes::exprs(self.cx, bytes),
                    };
                    push_variant(&mut variants, (state, value), limit)?;
                }
                variants
            }
            DynSolType::String => {
                let mut state = state;
                let lengths = self.next_dynamic_length_options(
                    &mut state,
                    &name,
                    &aliases,
                    DynamicKind::String,
                )?;
                let limit = calldata_variant_limit(self.config);
                let mut variants = Vec::new();
                for len in lengths {
                    let mut state = state.clone();
                    let bytes = (0..len as usize)
                        .map(|idx| self.fresh_byte(&mut state, &format!("{name}_{idx}"), true))
                        .collect();
                    let value = SymbolicAbiValue::String { bytes: SymBytes::exprs(self.cx, bytes) };
                    push_variant(&mut variants, (state, value), limit)?;
                }
                variants
            }
            DynSolType::Array(inner) => {
                let mut state = state;
                let lengths = self.next_dynamic_length_options(
                    &mut state,
                    &name,
                    &aliases,
                    DynamicKind::Array,
                )?;
                let limit = calldata_variant_limit(self.config);
                let mut variants = Vec::new();
                for len in lengths {
                    for (state, elements) in self.array_elements_variants(
                        state.clone(),
                        &name,
                        &aliases,
                        inner,
                        len as usize,
                    )? {
                        push_variant(
                            &mut variants,
                            (state, SymbolicAbiValue::Array { elements }),
                            limit,
                        )?;
                    }
                }
                variants
            }
            DynSolType::FixedArray(inner, len) => self
                .array_elements_variants(state, &name, &aliases, inner, *len)
                .map(|variants| {
                    variants
                        .into_iter()
                        .map(|(state, elements)| (state, SymbolicAbiValue::FixedArray { elements }))
                        .collect()
                })?,
            DynSolType::Tuple(types) => self
                .tuple_elements_variants(state, &name, &aliases, types)?
                .into_iter()
                .map(|(state, elements)| (state, SymbolicAbiValue::Tuple { elements }))
                .collect(),
            DynSolType::CustomStruct { tuple, .. } => self
                .tuple_elements_variants(state, &name, &aliases, tuple)?
                .into_iter()
                .map(|(state, elements)| (state, SymbolicAbiValue::Tuple { elements }))
                .collect(),
            _ => {
                let mut state = state;
                let value = self.value(&mut state, name, aliases, ty)?;
                vec![(state, value)]
            }
        })
    }

    pub(super) fn array_elements_variants(
        &mut self,
        state: SymbolicAbiState,
        name: &str,
        aliases: &[String],
        inner: &DynSolType,
        len: usize,
    ) -> Result<Vec<(SymbolicAbiState, Vec<SymbolicAbiValue>)>, SymbolicError> {
        let limit = calldata_variant_limit(self.config);
        let mut variants = vec![(state, Vec::with_capacity(len))];
        for idx in 0..len {
            let mut next_variants = Vec::new();
            for (state, elements) in variants {
                for (state, value) in self.value_variants(
                    state,
                    format!("{name}_{idx}"),
                    child_aliases(aliases, idx),
                    inner,
                )? {
                    let mut elements = elements.clone();
                    elements.push(value);
                    push_variant(&mut next_variants, (state, elements), limit)?;
                }
            }
            variants = next_variants;
        }
        Ok(variants)
    }

    pub(super) fn tuple_elements_variants(
        &mut self,
        state: SymbolicAbiState,
        name: &str,
        aliases: &[String],
        types: &[DynSolType],
    ) -> Result<Vec<(SymbolicAbiState, Vec<SymbolicAbiValue>)>, SymbolicError> {
        let limit = calldata_variant_limit(self.config);
        let mut variants = vec![(state, Vec::with_capacity(types.len()))];
        for (idx, ty) in types.iter().enumerate() {
            let mut next_variants = Vec::new();
            for (state, elements) in variants {
                for (state, value) in self.value_variants(
                    state,
                    format!("{name}_{idx}"),
                    child_aliases(aliases, idx),
                    ty,
                )? {
                    let mut elements = elements.clone();
                    elements.push(value);
                    push_variant(&mut next_variants, (state, elements), limit)?;
                }
            }
            variants = next_variants;
        }
        Ok(variants)
    }

    pub(super) fn fresh_word(&mut self, name: &str) -> SymExpr {
        self.cx.var(name)
    }

    pub(super) fn fresh_byte(
        &mut self,
        state: &mut SymbolicAbiState,
        name: &str,
        printable: bool,
    ) -> SymExpr {
        let word = self.fresh_word(name);
        state.constraints.push(self.cx.cmp_word_const(SymBoolExprOp::Ult, &word, U256::from(256)));
        if printable {
            state.constraints.push(self.cx.cmp_word_const(
                SymBoolExprOp::Uge,
                &word,
                U256::from(0x20),
            ));
            state.constraints.push(self.cx.cmp_word_const(
                SymBoolExprOp::Ule,
                &word,
                U256::from(0x7e),
            ));
        }
        word
    }

    pub(super) fn next_dynamic_length(
        &self,
        state: &mut SymbolicAbiState,
        name: &str,
        aliases: &[String],
        kind: DynamicKind,
    ) -> Result<usize, SymbolicError> {
        Ok(first_dynamic_length(
            &self.next_dynamic_length_options(state, name, aliases, kind)?,
            "symbolic dynamic length",
        )? as usize)
    }

    pub(super) fn next_dynamic_length_options(
        &self,
        state: &mut SymbolicAbiState,
        name: &str,
        aliases: &[String],
        kind: DynamicKind,
    ) -> Result<Vec<u32>, SymbolicError> {
        let named_lengths = std::iter::once(name)
            .chain(aliases.iter().map(String::as_str))
            .find_map(|name| self.config.dynamic_lengths.get(name));

        let lengths = if let Some(lengths) = named_lengths {
            lengths.clone()
        } else if let Some(lengths) = kind.default_lengths(self.config) {
            lengths.to_vec()
        } else if let Some(len) =
            self.config.array_lengths.get(state.positional_dynamic_index).copied()
        {
            state.positional_dynamic_index += 1;
            vec![len]
        } else {
            vec![self.config.default_dynamic_length]
        };

        if lengths.is_empty() {
            return Err(SymbolicError::UnsupportedAbi(
                "symbolic dynamic length set must not be empty".to_string(),
            ));
        }
        for len in &lengths {
            if *len > self.config.max_dynamic_length {
                return Err(SymbolicError::UnsupportedAbi(format!(
                    "symbolic {} length {len} exceeds max_dynamic_length {}",
                    kind.name(),
                    self.config.max_dynamic_length
                )));
            }
        }
        Ok(lengths)
    }

    pub(super) fn constrain_uint(
        &mut self,
        state: &mut SymbolicAbiState,
        word: &SymExpr,
        bits: usize,
    ) {
        if bits < 256 {
            state.constraints.push(self.cx.cmp_word_const(
                SymBoolExprOp::Ult,
                word,
                U256::from(1) << bits,
            ));
        }
    }

    pub(super) fn constrain_int(
        &mut self,
        state: &mut SymbolicAbiState,
        word: &SymExpr,
        bits: usize,
    ) {
        if bits < 256 {
            let byte_index = U256::from(bits / 8 - 1);
            let signextended = signextend_word(self.cx, byte_index, word.clone());
            state.constraints.push(self.cx.eq(word.clone(), signextended));
        }
    }

    pub(super) fn encode_sequence<'v>(
        &mut self,
        values: impl IntoIterator<Item = &'v SymbolicAbiValue>,
    ) -> SymBytes {
        SymbolicAbiEncoder { cx: self.cx }.encode_sequence(values)
    }
}

impl<'cx> SymbolicAbiEncoder<'cx> {
    fn encode_sequence<'v>(
        &mut self,
        values: impl IntoIterator<Item = &'v SymbolicAbiValue>,
    ) -> SymBytes {
        let values = values.into_iter().collect::<Vec<_>>();
        let head_size = values.iter().map(|value| value.head_size()).sum::<usize>();
        let mut head = Vec::with_capacity(values.len());
        let mut tail = Vec::new();
        let mut tail_len = 0usize;

        for value in values {
            if value.is_dynamic() {
                let offset = self.cx.constant(U256::from(head_size + tail_len));
                head.push(offset.into_bytes(self.cx));
                let body = self.encode_dynamic_body(value);
                tail_len += body.len();
                tail.push(body);
            } else {
                head.push(self.encode_static(value));
            }
        }

        SymBytes::concat(self.cx, head.into_iter().chain(tail))
    }

    fn encode_static(&mut self, value: &SymbolicAbiValue) -> SymBytes {
        match value {
            SymbolicAbiValue::Bool { word }
            | SymbolicAbiValue::Uint { word, .. }
            | SymbolicAbiValue::Int { word, .. }
            | SymbolicAbiValue::Address { word } => word.clone().into_bytes(self.cx),
            SymbolicAbiValue::FixedBytes { bytes, .. } => {
                let padding =
                    SymBytes::concrete(self.cx, vec![0; 32usize.saturating_sub(bytes.len())]);
                SymBytes::concat(self.cx, [bytes.clone(), padding])
            }
            SymbolicAbiValue::FixedArray { elements } | SymbolicAbiValue::Tuple { elements } => {
                self.encode_sequence(elements.iter())
            }
            SymbolicAbiValue::Bytes { .. }
            | SymbolicAbiValue::String { .. }
            | SymbolicAbiValue::Array { .. } => unreachable!("dynamic ABI value encoded as static"),
        }
    }

    fn encode_dynamic_body(&mut self, value: &SymbolicAbiValue) -> SymBytes {
        match value {
            SymbolicAbiValue::Bytes { len, bytes } => {
                encode_packed_bytes_with_len(self.cx, len.clone(), bytes)
            }
            SymbolicAbiValue::String { bytes } => {
                let len = self.cx.constant(U256::from(bytes.len()));
                encode_packed_bytes_with_len(self.cx, len, bytes)
            }
            SymbolicAbiValue::Array { elements } => {
                let len = self.cx.constant(U256::from(elements.len()));
                let len = len.into_bytes(self.cx);
                let elements = self.encode_sequence(elements.iter());
                SymBytes::concat(self.cx, [len, elements])
            }
            SymbolicAbiValue::FixedArray { elements } | SymbolicAbiValue::Tuple { elements } => {
                self.encode_sequence(elements.iter())
            }
            SymbolicAbiValue::Bool { .. }
            | SymbolicAbiValue::Uint { .. }
            | SymbolicAbiValue::Int { .. }
            | SymbolicAbiValue::FixedBytes { .. }
            | SymbolicAbiValue::Address { .. } => {
                unreachable!("static ABI value encoded as dynamic")
            }
        }
    }
}

/// Validates that positional ABI length config can be consumed by at least one expanded variant.
fn validate_positional_dynamic_lengths(
    config: &SymbolicConfig,
    max_positional_dynamic_index: usize,
) -> Result<(), SymbolicError> {
    if config.array_lengths.len() > max_positional_dynamic_index {
        return Err(SymbolicError::UnsupportedAbi(format!(
            "symbolic.array_lengths has {} entries but ABI used at most {} positional dynamic leaves",
            config.array_lengths.len(),
            max_positional_dynamic_index
        )));
    }
    Ok(())
}

/// Returns the maximum number of calldata variants allowed during ABI expansion.
fn calldata_variant_limit(config: &SymbolicConfig) -> usize {
    config.path_width().max(1) as usize
}

/// Adds one expansion variant while enforcing the configured symbolic path-width budget.
fn push_variant<T>(variants: &mut Vec<T>, variant: T, limit: usize) -> Result<(), SymbolicError> {
    if variants.len() >= limit {
        return Err(SymbolicError::CalldataVariantLimit(limit));
    }
    variants.push(variant);
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum DynamicKind {
    Array,
    Bytes,
    String,
}

impl DynamicKind {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Array => "array",
            Self::Bytes => "bytes",
            Self::String => "string",
        }
    }

    pub(super) fn default_lengths(self, config: &SymbolicConfig) -> Option<&[u32]> {
        match self {
            Self::Array if !config.default_array_lengths.is_empty() => {
                Some(&config.default_array_lengths)
            }
            Self::Bytes | Self::String if !config.default_bytes_lengths.is_empty() => {
                Some(&config.default_bytes_lengths)
            }
            _ => None,
        }
    }
}

pub(super) fn first_dynamic_length(lengths: &[u32], field: &str) -> Result<u32, SymbolicError> {
    lengths
        .first()
        .copied()
        .ok_or_else(|| SymbolicError::UnsupportedAbi(format!("{field} must not be empty")))
}

pub(super) fn child_aliases(aliases: &[String], idx: usize) -> Vec<String> {
    aliases.iter().map(|alias| format!("{alias}_{idx}")).collect()
}

#[derive(Clone, Debug)]
pub(super) enum SymbolicAbiValue {
    Bool { word: SymExpr },
    Uint { bits: usize, word: SymExpr },
    Int { bits: usize, word: SymExpr },
    FixedBytes { bytes: SymBytes, size: usize },
    Address { word: SymExpr },
    Bytes { len: SymExpr, bytes: SymBytes },
    String { bytes: SymBytes },
    Array { elements: Vec<Self> },
    FixedArray { elements: Vec<Self> },
    Tuple { elements: Vec<Self> },
}

impl SymbolicAbiValue {
    /// Returns whether `is_dynamic` holds.
    pub(super) fn is_dynamic(&self) -> bool {
        match self {
            Self::Bool { .. }
            | Self::Uint { .. }
            | Self::Int { .. }
            | Self::FixedBytes { .. }
            | Self::Address { .. } => false,
            Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => true,
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                elements.iter().any(Self::is_dynamic)
            }
        }
    }

    pub(super) fn head_size(&self) -> usize {
        if self.is_dynamic() {
            32
        } else {
            match self {
                Self::Bool { .. }
                | Self::Uint { .. }
                | Self::Int { .. }
                | Self::FixedBytes { .. }
                | Self::Address { .. } => 32,
                Self::FixedArray { elements } | Self::Tuple { elements } => {
                    elements.iter().map(Self::head_size).sum()
                }
                Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => 32,
            }
        }
    }

    pub(super) fn model_value(
        &self,
        cx: &mut SymCx,
        model: &(impl SymbolicModelLookup + ?Sized),
    ) -> Result<DynSolValue, SymbolicError> {
        Ok(match self {
            Self::Bool { word } => DynSolValue::Bool(!word.eval_model(model)?.is_zero()),
            Self::Uint { bits, word } => {
                DynSolValue::Uint(mask_bits(word.eval_model(model)?, *bits), *bits)
            }
            Self::Int { bits, word } => {
                DynSolValue::Int(I256::from_raw(word.eval_model(model)?), *bits)
            }
            Self::FixedBytes { bytes, size } => {
                let mut word = [0u8; 32];
                for (idx, out) in word.iter_mut().enumerate().take(bytes.len()) {
                    *out = bytes.byte(cx, idx).eval_model(model)?.to::<u8>();
                }
                DynSolValue::FixedBytes(B256::from(word), *size)
            }
            Self::Address { word } => {
                DynSolValue::Address(word_to_address(word.eval_model(model)?))
            }
            Self::Bytes { len, bytes } => {
                let len = len.eval_model(model)?;
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= bytes.len())
                    .ok_or_else(|| SymbolicError::Solver("invalid symbolic bytes length".into()))?;
                let mut bytes = bytes.eval_model(cx, model)?;
                bytes.truncate(len);
                DynSolValue::Bytes(bytes)
            }
            Self::String { bytes } => {
                let bytes = bytes.eval_model(cx, model)?;
                let value = String::from_utf8(bytes).map_err(|err| {
                    SymbolicError::Solver(format!("invalid symbolic string model: {err}"))
                })?;
                DynSolValue::String(value)
            }
            Self::Array { elements } => DynSolValue::Array(
                elements
                    .iter()
                    .map(|value| value.model_value(cx, model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::FixedArray { elements } => DynSolValue::FixedArray(
                elements
                    .iter()
                    .map(|value| value.model_value(cx, model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::Tuple { elements } => DynSolValue::Tuple(
                elements
                    .iter()
                    .map(|value| value.model_value(cx, model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        })
    }

    pub(super) fn seed_model_value(
        &self,
        cx: &mut SymCx,
        model: &mut SymbolicModel,
        value: &DynSolValue,
    ) -> bool {
        match (self, value) {
            (Self::Bool { word }, DynSolValue::Bool(value)) => {
                word.assign_model_value(model, U256::from(*value as u8))
            }
            (Self::Uint { bits, word }, DynSolValue::Uint(value, value_bits))
                if bits == value_bits =>
            {
                word.assign_model_value(model, *value)
            }
            (Self::Int { bits, word }, DynSolValue::Int(value, value_bits))
                if bits == value_bits =>
            {
                word.assign_model_value(model, value.into_raw())
            }
            (Self::FixedBytes { bytes, size }, DynSolValue::FixedBytes(value, value_size))
                if size == value_size =>
            {
                seed_model_bytes(cx, model, bytes, &value.as_slice()[..*size])
            }
            (Self::Address { word }, DynSolValue::Address(value)) => {
                word.assign_model_value(model, address_word(*value))
            }
            (Self::Bytes { len, bytes }, DynSolValue::Bytes(value)) => {
                len.assign_model_value(model, U256::from(value.len()))
                    && seed_model_bytes(cx, model, bytes, value)
            }
            (Self::String { bytes }, DynSolValue::String(value)) => {
                seed_model_bytes(cx, model, bytes, value.as_bytes())
            }
            (Self::Array { elements }, DynSolValue::Array(values))
            | (Self::FixedArray { elements }, DynSolValue::FixedArray(values))
            | (Self::Tuple { elements }, DynSolValue::Tuple(values)) => {
                seed_model_elements(cx, model, elements, values)
            }
            (Self::Tuple { elements }, DynSolValue::CustomStruct { tuple, .. }) => {
                seed_model_elements(cx, model, elements, tuple)
            }
            _ => false,
        }
    }
}

fn seed_model_elements(
    cx: &mut SymCx,
    model: &mut SymbolicModel,
    elements: &[SymbolicAbiValue],
    values: &[DynSolValue],
) -> bool {
    elements.len() == values.len()
        && elements
            .iter()
            .zip(values)
            .all(|(element, value)| element.seed_model_value(cx, model, value))
}

fn seed_model_bytes(
    cx: &mut SymCx,
    model: &mut SymbolicModel,
    bytes: &SymBytes,
    value: &[u8],
) -> bool {
    bytes.len() == value.len()
        && value
            .iter()
            .enumerate()
            .all(|(idx, byte)| bytes.byte(cx, idx).assign_model_value(model, U256::from(*byte)))
}

pub(super) fn encode_sequence<'a>(
    cx: &mut SymCx,
    values: impl IntoIterator<Item = &'a SymbolicAbiValue>,
) -> SymBytes {
    SymbolicAbiEncoder { cx }.encode_sequence(values)
}

pub(super) fn encode_packed_bytes_with_len(
    cx: &mut SymCx,
    len: SymExpr,
    bytes: &SymBytes,
) -> SymBytes {
    let padded_len = bytes.len().next_multiple_of(32);
    let len = len.into_bytes(cx);
    let padding = SymBytes::concrete(cx, vec![0; padded_len - bytes.len()]);
    SymBytes::concat(cx, [len, bytes.clone(), padding])
}
