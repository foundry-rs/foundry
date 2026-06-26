use super::{runtime::*, *};

#[derive(Clone, Debug)]
pub(super) struct SymbolicCalldata {
    bytes: Arc<[SymWord]>,
    inputs: Vec<SymbolicInput>,
    constraints: Vec<BoolExpr>,
}

impl SymbolicCalldata {
    /// Constructs a raw symbolic calldata fixture.
    #[cfg(test)]
    pub(super) fn from_raw(
        bytes: Vec<SymWord>,
        inputs: Vec<SymbolicInput>,
        constraints: Vec<BoolExpr>,
    ) -> Self {
        Self { bytes: bytes.into(), inputs, constraints }
    }

    /// Constructs a new instance.
    #[cfg(test)]
    pub(super) fn new(function: &Function, config: &SymbolicConfig) -> Result<Self, SymbolicError> {
        Ok(Self::variants(function, config)?.remove(0))
    }

    pub(super) fn variants(
        function: &Function,
        config: &SymbolicConfig,
    ) -> Result<Vec<Self>, SymbolicError> {
        Self::variants_with_prefix(function, config, "calldata")
    }

    pub(super) fn selector_only(function: &Function) -> Result<Self, SymbolicError> {
        if !function.inputs.is_empty() {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic invariant `{}` must take no parameters",
                function.name
            )));
        }
        let bytes = function
            .selector()
            .iter()
            .copied()
            .map(|byte| SymWord::constant(U256::from(byte)))
            .collect::<Vec<_>>();
        Ok(Self { bytes: bytes.into(), inputs: Vec::new(), constraints: Vec::new() })
    }

    pub(super) fn variants_with_prefix(
        function: &Function,
        config: &SymbolicConfig,
        prefix: impl AsRef<str>,
    ) -> Result<Vec<Self>, SymbolicError> {
        let prefix = prefix.as_ref();
        let variant_limit = calldata_variant_limit(config);
        let mut variants = vec![(SymbolicAbiBuilder::new(config), Vec::new())];
        for (idx, input) in function.inputs.iter().enumerate() {
            let ty = input.selector_type();
            let mut next_variants = Vec::new();
            for (builder, inputs) in variants {
                for (builder, input) in SymbolicInput::variants(
                    builder,
                    prefix,
                    idx,
                    Some(input.name.as_str()),
                    ty.as_ref(),
                )? {
                    let mut inputs = inputs.clone();
                    inputs.push(input);
                    push_variant(&mut next_variants, (builder, inputs), variant_limit)?;
                }
            }
            variants = next_variants;
        }

        validate_positional_dynamic_lengths(
            config,
            variants.iter().map(|(builder, _)| builder.positional_dynamic_index).max().unwrap_or(0),
        )?;

        variants
            .into_iter()
            .map(|(builder, inputs)| {
                let mut bytes = function
                    .selector()
                    .iter()
                    .copied()
                    .map(|byte| SymWord::constant(U256::from(byte)))
                    .collect::<Vec<_>>();
                bytes.extend(encode_sequence(inputs.iter().map(|input| &input.value)));
                if bytes.len() > config.max_calldata_bytes as usize {
                    return Err(SymbolicError::Unsupported(
                        "symbolic calldata size exceeds configured max",
                    ));
                }

                Ok(Self { bytes: bytes.into(), inputs, constraints: builder.constraints })
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    #[cfg(test)]
    pub(super) fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    pub(super) fn call_data(&self) -> SymCalldata {
        SymCalldata::from_shared(self.bytes.clone())
    }

    /// Returns symbolic calldata constraints.
    pub(super) fn constraints(&self) -> &[BoolExpr] {
        &self.constraints
    }

    /// Consumes this symbolic calldata into its constraints.
    pub(super) fn into_constraints(self) -> Vec<BoolExpr> {
        self.constraints
    }

    /// Returns the encoded symbolic calldata length.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns symbolic ABI inputs.
    #[cfg(test)]
    pub(super) fn inputs(&self) -> &[SymbolicInput] {
        &self.inputs
    }

    pub(super) fn model_to_args(
        &self,
        model: &(impl SymbolicModelLookup + ?Sized),
    ) -> Result<Vec<DynSolValue>, SymbolicError> {
        self.inputs.iter().map(|input| input.value.model_value(model)).collect()
    }
}

#[derive(Clone, Debug)]
pub(super) struct SymbolicInput {
    value: SymbolicAbiValue,
}

impl SymbolicInput {
    /// Returns this symbolic input value.
    #[cfg(test)]
    pub(super) const fn value(&self) -> &SymbolicAbiValue {
        &self.value
    }

    pub(super) fn variants<'a>(
        builder: SymbolicAbiBuilder<'a>,
        prefix: &str,
        idx: usize,
        abi_name: Option<&str>,
        ty: &str,
    ) -> Result<Vec<(SymbolicAbiBuilder<'a>, Self)>, SymbolicError> {
        let ty =
            DynSolType::parse(ty).map_err(|_| SymbolicError::UnsupportedAbi(ty.to_string()))?;
        let name = format!("{prefix}_{idx}");
        let aliases =
            abi_name.filter(|name| !name.is_empty()).map(str::to_string).into_iter().collect();
        builder.value_variants(name, aliases, &ty).map(|variants| {
            variants.into_iter().map(|(builder, value)| (builder, Self { value })).collect()
        })
    }
}

#[derive(Clone)]
pub(super) struct SymbolicAbiBuilder<'a> {
    config: &'a SymbolicConfig,
    constraints: Vec<BoolExpr>,
    positional_dynamic_index: usize,
}

impl<'a> SymbolicAbiBuilder<'a> {
    /// Constructs a new instance.
    pub(super) const fn new(config: &'a SymbolicConfig) -> Self {
        Self { config, constraints: Vec::new(), positional_dynamic_index: 0 }
    }

    pub(super) fn value(
        &mut self,
        name: String,
        aliases: Vec<String>,
        ty: &DynSolType,
    ) -> Result<SymbolicAbiValue, SymbolicError> {
        Ok(match ty {
            DynSolType::Bool => {
                let word = self.fresh_word(name);
                self.constraints.push(BoolExpr::cmp_word_const(
                    BoolExprOp::Ult,
                    &word,
                    U256::from(2),
                ));
                SymbolicAbiValue::Bool { word }
            }
            DynSolType::Uint(bits) => {
                let word = self.fresh_word(name);
                self.constrain_uint(&word, *bits);
                SymbolicAbiValue::Uint { bits: *bits, word }
            }
            DynSolType::Int(bits) => {
                let word = self.fresh_word(name);
                self.constrain_int(&word, *bits);
                SymbolicAbiValue::Int { bits: *bits, word }
            }
            DynSolType::FixedBytes(size) => SymbolicAbiValue::FixedBytes {
                bytes: (0..*size)
                    .map(|idx| self.fresh_byte(format!("{name}_{idx}"), false))
                    .collect::<Vec<_>>()
                    .into(),
                size: *size,
            },
            DynSolType::Address => {
                let word = self.fresh_word(name);
                self.constrain_uint(&word, 160);
                SymbolicAbiValue::Address { word }
            }
            DynSolType::Function => {
                return Err(SymbolicError::UnsupportedAbi("function".to_string()));
            }
            DynSolType::Bytes => {
                let len = self.next_dynamic_length(&name, &aliases, DynamicKind::Bytes)?;
                SymbolicAbiValue::Bytes {
                    len: SymWord::constant(U256::from(len)),
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), false))
                        .collect::<Vec<_>>()
                        .into(),
                }
            }
            DynSolType::String => {
                let len = self.next_dynamic_length(&name, &aliases, DynamicKind::String)?;
                SymbolicAbiValue::String {
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), true))
                        .collect::<Vec<_>>()
                        .into(),
                }
            }
            DynSolType::Array(inner) => {
                let len = self.next_dynamic_length(&name, &aliases, DynamicKind::Array)?;
                SymbolicAbiValue::Array {
                    elements: (0..len)
                        .map(|idx| {
                            self.value(format!("{name}_{idx}"), child_aliases(&aliases, idx), inner)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            DynSolType::FixedArray(inner, len) => SymbolicAbiValue::FixedArray {
                elements: (0..*len)
                    .map(|idx| {
                        self.value(format!("{name}_{idx}"), child_aliases(&aliases, idx), inner)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::Tuple(types) => SymbolicAbiValue::Tuple {
                elements: types
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| {
                        self.value(format!("{name}_{idx}"), child_aliases(&aliases, idx), ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::CustomStruct { tuple, .. } => SymbolicAbiValue::Tuple {
                elements: tuple
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| {
                        self.value(format!("{name}_{idx}"), child_aliases(&aliases, idx), ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })
    }

    pub(super) fn value_variants(
        self,
        name: String,
        aliases: Vec<String>,
        ty: &DynSolType,
    ) -> Result<Vec<(Self, SymbolicAbiValue)>, SymbolicError> {
        Ok(match ty {
            DynSolType::Bytes => {
                let mut builder = self;
                let lengths =
                    builder.next_dynamic_length_options(&name, &aliases, DynamicKind::Bytes)?;
                let limit = calldata_variant_limit(builder.config);
                let mut variants = Vec::new();
                for len in lengths {
                    let mut builder = builder.clone();
                    let value = SymbolicAbiValue::Bytes {
                        len: SymWord::constant(U256::from(len)),
                        bytes: (0..len as usize)
                            .map(|idx| builder.fresh_byte(format!("{name}_{idx}"), false))
                            .collect::<Vec<_>>()
                            .into(),
                    };
                    push_variant(&mut variants, (builder, value), limit)?;
                }
                variants
            }
            DynSolType::String => {
                let mut builder = self;
                let lengths =
                    builder.next_dynamic_length_options(&name, &aliases, DynamicKind::String)?;
                let limit = calldata_variant_limit(builder.config);
                let mut variants = Vec::new();
                for len in lengths {
                    let mut builder = builder.clone();
                    let value = SymbolicAbiValue::String {
                        bytes: (0..len as usize)
                            .map(|idx| builder.fresh_byte(format!("{name}_{idx}"), true))
                            .collect::<Vec<_>>()
                            .into(),
                    };
                    push_variant(&mut variants, (builder, value), limit)?;
                }
                variants
            }
            DynSolType::Array(inner) => {
                let mut builder = self;
                let lengths =
                    builder.next_dynamic_length_options(&name, &aliases, DynamicKind::Array)?;
                let limit = calldata_variant_limit(builder.config);
                let mut variants = Vec::new();
                for len in lengths {
                    for (builder, elements) in builder.clone().array_elements_variants(
                        &name,
                        &aliases,
                        inner,
                        len as usize,
                    )? {
                        push_variant(
                            &mut variants,
                            (builder, SymbolicAbiValue::Array { elements }),
                            limit,
                        )?;
                    }
                }
                variants
            }
            DynSolType::FixedArray(inner, len) => {
                self.array_elements_variants(&name, &aliases, inner, *len).map(|variants| {
                    variants
                        .into_iter()
                        .map(|(builder, elements)| {
                            (builder, SymbolicAbiValue::FixedArray { elements })
                        })
                        .collect()
                })?
            }
            DynSolType::Tuple(types) => self
                .tuple_elements_variants(&name, &aliases, types)?
                .into_iter()
                .map(|(builder, elements)| (builder, SymbolicAbiValue::Tuple { elements }))
                .collect(),
            DynSolType::CustomStruct { tuple, .. } => self
                .tuple_elements_variants(&name, &aliases, tuple)?
                .into_iter()
                .map(|(builder, elements)| (builder, SymbolicAbiValue::Tuple { elements }))
                .collect(),
            _ => {
                let mut builder = self;
                let value = builder.value(name, aliases, ty)?;
                vec![(builder, value)]
            }
        })
    }

    pub(super) fn array_elements_variants(
        self,
        name: &str,
        aliases: &[String],
        inner: &DynSolType,
        len: usize,
    ) -> Result<Vec<(Self, Vec<SymbolicAbiValue>)>, SymbolicError> {
        let limit = calldata_variant_limit(self.config);
        let mut variants = vec![(self, Vec::with_capacity(len))];
        for idx in 0..len {
            let mut next_variants = Vec::new();
            for (builder, elements) in variants {
                for (builder, value) in builder.value_variants(
                    format!("{name}_{idx}"),
                    child_aliases(aliases, idx),
                    inner,
                )? {
                    let mut elements = elements.clone();
                    elements.push(value);
                    push_variant(&mut next_variants, (builder, elements), limit)?;
                }
            }
            variants = next_variants;
        }
        Ok(variants)
    }

    pub(super) fn tuple_elements_variants(
        self,
        name: &str,
        aliases: &[String],
        types: &[DynSolType],
    ) -> Result<Vec<(Self, Vec<SymbolicAbiValue>)>, SymbolicError> {
        let limit = calldata_variant_limit(self.config);
        let mut variants = vec![(self, Vec::with_capacity(types.len()))];
        for (idx, ty) in types.iter().enumerate() {
            let mut next_variants = Vec::new();
            for (builder, elements) in variants {
                for (builder, value) in builder.value_variants(
                    format!("{name}_{idx}"),
                    child_aliases(aliases, idx),
                    ty,
                )? {
                    let mut elements = elements.clone();
                    elements.push(value);
                    push_variant(&mut next_variants, (builder, elements), limit)?;
                }
            }
            variants = next_variants;
        }
        Ok(variants)
    }

    pub(super) fn fresh_word(&self, name: String) -> SymWord {
        SymWord::expr(Expr::var(name))
    }

    pub(super) fn fresh_byte(&mut self, name: String, printable: bool) -> SymWord {
        let word = self.fresh_word(name);
        self.constraints.push(BoolExpr::cmp_word_const(BoolExprOp::Ult, &word, U256::from(256)));
        if printable {
            self.constraints.push(BoolExpr::cmp_word_const(
                BoolExprOp::Uge,
                &word,
                U256::from(0x20),
            ));
            self.constraints.push(BoolExpr::cmp_word_const(
                BoolExprOp::Ule,
                &word,
                U256::from(0x7e),
            ));
        }
        word
    }

    pub(super) fn next_dynamic_length(
        &mut self,
        name: &str,
        aliases: &[String],
        kind: DynamicKind,
    ) -> Result<usize, SymbolicError> {
        Ok(first_dynamic_length(
            &self.next_dynamic_length_options(name, aliases, kind)?,
            "symbolic dynamic length",
        )? as usize)
    }

    pub(super) fn next_dynamic_length_options(
        &mut self,
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
            self.config.array_lengths.get(self.positional_dynamic_index).copied()
        {
            self.positional_dynamic_index += 1;
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

    pub(super) fn constrain_uint(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            self.constraints.push(BoolExpr::cmp_word_const(
                BoolExprOp::Ult,
                word,
                U256::from(1) << bits,
            ));
        }
    }

    pub(super) fn constrain_int(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            let byte_index = U256::from(bits / 8 - 1);
            self.constraints.push(BoolExpr::eq_word_expr(
                word,
                signextend_word(byte_index, word.clone()).into_expr(),
            ));
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
    Bool { word: SymWord },
    Uint { bits: usize, word: SymWord },
    Int { bits: usize, word: SymWord },
    FixedBytes { bytes: Arc<[SymWord]>, size: usize },
    Address { word: SymWord },
    Bytes { len: SymWord, bytes: Arc<[SymWord]> },
    String { bytes: Arc<[SymWord]> },
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

    pub(super) fn encode_static(&self) -> Vec<SymWord> {
        match self {
            Self::Bool { word }
            | Self::Uint { word, .. }
            | Self::Int { word, .. }
            | Self::Address { word } => word_bytes(word.clone()),
            Self::FixedBytes { bytes, .. } => {
                let mut out = bytes.to_vec();
                out.resize(32, SymWord::zero());
                out
            }
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                encode_sequence(elements.iter())
            }
            Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => {
                unreachable!("dynamic ABI value encoded as static")
            }
        }
    }

    pub(super) fn encode_dynamic_body(&self) -> Vec<SymWord> {
        match self {
            Self::Bytes { len, bytes } => encode_packed_bytes_with_len(len.clone(), bytes),
            Self::String { bytes } => {
                encode_packed_bytes_with_len(SymWord::constant(U256::from(bytes.len())), bytes)
            }
            Self::Array { elements } => {
                let mut out = word_bytes(SymWord::constant(U256::from(elements.len())));
                out.extend(encode_sequence(elements.iter()));
                out
            }
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                encode_sequence(elements.iter())
            }
            Self::Bool { .. }
            | Self::Uint { .. }
            | Self::Int { .. }
            | Self::FixedBytes { .. }
            | Self::Address { .. } => unreachable!("static ABI value encoded as dynamic"),
        }
    }

    pub(super) fn model_value(
        &self,
        model: &(impl SymbolicModelLookup + ?Sized),
    ) -> Result<DynSolValue, SymbolicError> {
        Ok(match self {
            Self::Bool { word } => DynSolValue::Bool(!word.eval(model)?.is_zero()),
            Self::Uint { bits, word } => {
                DynSolValue::Uint(mask_bits(word.eval(model)?, *bits), *bits)
            }
            Self::Int { bits, word } => DynSolValue::Int(I256::from_raw(word.eval(model)?), *bits),
            Self::FixedBytes { bytes, size } => {
                let mut word = [0u8; 32];
                for (idx, byte) in bytes.iter().enumerate() {
                    word[idx] = byte.eval(model)?.to::<u8>();
                }
                DynSolValue::FixedBytes(B256::from(word), *size)
            }
            Self::Address { word } => DynSolValue::Address(word_to_address(word.eval(model)?)),
            Self::Bytes { len, bytes } => {
                let len = len.eval(model)?;
                let len = u256_to_usize(len)
                    .filter(|len| *len <= bytes.len())
                    .ok_or_else(|| SymbolicError::Solver("invalid symbolic bytes length".into()))?;
                let mut bytes = bytes.eval(model)?;
                bytes.truncate(len);
                DynSolValue::Bytes(bytes)
            }
            Self::String { bytes } => {
                let bytes = bytes.eval(model)?;
                let value = String::from_utf8(bytes).map_err(|err| {
                    SymbolicError::Solver(format!("invalid symbolic string model: {err}"))
                })?;
                DynSolValue::String(value)
            }
            Self::Array { elements } => DynSolValue::Array(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::FixedArray { elements } => DynSolValue::FixedArray(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::Tuple { elements } => DynSolValue::Tuple(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        })
    }
}

pub(super) fn encode_sequence<'a>(
    values: impl IntoIterator<Item = &'a SymbolicAbiValue>,
) -> Vec<SymWord> {
    let values = values.into_iter().collect::<Vec<_>>();
    let head_size = values.iter().map(|value| value.head_size()).sum::<usize>();
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();

    for value in values {
        if value.is_dynamic() {
            head.extend(word_bytes(SymWord::constant(U256::from(head_size + tail.len()))));
            tail.extend(value.encode_dynamic_body());
        } else {
            head.extend(value.encode_static());
        }
    }

    head.extend(tail);
    head
}

pub(super) fn encode_packed_bytes_with_len(len: SymWord, bytes: &[SymWord]) -> Vec<SymWord> {
    let mut out = word_bytes(len);
    out.extend(bytes.iter().cloned());
    out.resize(32 + bytes.len().next_multiple_of(32), SymWord::zero());
    out
}
