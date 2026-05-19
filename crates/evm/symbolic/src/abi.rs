use super::{runtime::*, *};

#[derive(Clone, Debug)]
pub(super) struct SymbolicCalldata {
    pub(super) size: usize,
    pub(super) bytes: Vec<SymWord>,
    pub(super) inputs: Vec<SymbolicInput>,
    pub(super) constraints: Vec<BoolExpr>,
}

impl SymbolicCalldata {
    /// Constructs a new instance.
    pub(super) fn new(function: &Function, config: &SymbolicConfig) -> Result<Self, SymbolicError> {
        Self::new_with_prefix(function, config, "calldata")
    }

    /// Returns the `selector_only` symbolic ABI helper result.
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
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect::<Vec<_>>();
        Ok(Self { size: bytes.len(), bytes, inputs: Vec::new(), constraints: Vec::new() })
    }

    /// Implements the `new_with_prefix` symbolic ABI helper.
    pub(super) fn new_with_prefix(
        function: &Function,
        config: &SymbolicConfig,
        prefix: impl AsRef<str>,
    ) -> Result<Self, SymbolicError> {
        let mut builder = SymbolicAbiBuilder::new(config);
        let mut inputs = Vec::with_capacity(function.inputs.len());
        for (idx, input) in function.inputs.iter().enumerate() {
            let ty = input.selector_type();
            inputs.push(SymbolicInput::new(&mut builder, prefix.as_ref(), idx, ty.as_ref())?);
        }
        builder.finish()?;

        let mut bytes = function
            .selector()
            .iter()
            .copied()
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect::<Vec<_>>();
        bytes.extend(encode_sequence(inputs.iter().map(|input| &input.value)));
        if bytes.len() > config.max_calldata_bytes as usize {
            return Err(SymbolicError::Unsupported(
                "symbolic calldata size exceeds configured max",
            ));
        }

        Ok(Self { size: bytes.len(), bytes, inputs, constraints: builder.constraints })
    }

    #[cfg(test)]
    /// Implements the `load` symbolic ABI helper.
    pub(super) fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    #[cfg(test)]
    /// Implements the `byte` symbolic ABI helper.
    pub(super) fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    /// Implements the `call_data` symbolic ABI helper.
    pub(super) fn call_data(&self) -> SymCalldata {
        SymCalldata {
            size: self.size,
            size_word: SymWord::Concrete(U256::from(self.size)),
            bytes: self.bytes.clone(),
        }
    }

    /// Returns the `model_to_args` symbolic ABI helper result.
    pub(super) fn model_to_args(
        &self,
        model: &BTreeMap<String, U256>,
    ) -> Result<Vec<DynSolValue>, SymbolicError> {
        self.inputs.iter().map(|input| input.value.model_value(model)).collect()
    }
}

#[derive(Clone, Debug)]
pub(super) struct SymbolicInput {
    pub(super) value: SymbolicAbiValue,
}

impl SymbolicInput {
    /// Constructs a new instance.
    pub(super) fn new(
        builder: &mut SymbolicAbiBuilder<'_>,
        prefix: &str,
        idx: usize,
        ty: &str,
    ) -> Result<Self, SymbolicError> {
        let ty =
            DynSolType::parse(ty).map_err(|_| SymbolicError::UnsupportedAbi(ty.to_string()))?;
        Ok(Self { value: builder.value(format!("{prefix}_{idx}"), &ty)? })
    }
}

pub(super) struct SymbolicAbiBuilder<'a> {
    pub(super) config: &'a SymbolicConfig,
    pub(super) constraints: Vec<BoolExpr>,
    pub(super) dynamic_index: usize,
}

impl<'a> SymbolicAbiBuilder<'a> {
    /// Constructs a new instance.
    pub(super) const fn new(config: &'a SymbolicConfig) -> Self {
        Self { config, constraints: Vec::new(), dynamic_index: 0 }
    }

    /// Validates the `finish` symbolic ABI helper.
    pub(super) fn finish(&self) -> Result<(), SymbolicError> {
        if self.dynamic_index != self.config.array_lengths.len() {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic.array_lengths has {} entries but ABI has {} dynamic leaves",
                self.config.array_lengths.len(),
                self.dynamic_index
            )));
        }
        Ok(())
    }

    /// Implements the `value` symbolic ABI helper.
    pub(super) fn value(
        &mut self,
        name: String,
        ty: &DynSolType,
    ) -> Result<SymbolicAbiValue, SymbolicError> {
        Ok(match ty {
            DynSolType::Bool => {
                let word = self.fresh_word(name);
                self.constraints.push(BoolExpr::cmp(
                    BoolExprOp::Ult,
                    word.clone().into_expr(),
                    Expr::Const(U256::from(2)),
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
                    .collect(),
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
                let len = self.next_dynamic_length("bytes")?;
                SymbolicAbiValue::Bytes {
                    len: SymWord::Concrete(U256::from(len)),
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), false))
                        .collect(),
                }
            }
            DynSolType::String => {
                let len = self.next_dynamic_length("string")?;
                SymbolicAbiValue::String {
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), true))
                        .collect(),
                }
            }
            DynSolType::Array(inner) => {
                let len = self.next_dynamic_length("array")?;
                SymbolicAbiValue::Array {
                    elements: (0..len)
                        .map(|idx| self.value(format!("{name}_{idx}"), inner))
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            DynSolType::FixedArray(inner, len) => SymbolicAbiValue::FixedArray {
                elements: (0..*len)
                    .map(|idx| self.value(format!("{name}_{idx}"), inner))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::Tuple(types) => SymbolicAbiValue::Tuple {
                elements: types
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| self.value(format!("{name}_{idx}"), ty))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::CustomStruct { tuple, .. } => SymbolicAbiValue::Tuple {
                elements: tuple
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| self.value(format!("{name}_{idx}"), ty))
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })
    }

    /// Implements the `fresh_word` symbolic ABI helper.
    pub(super) const fn fresh_word(&self, name: String) -> SymWord {
        SymWord::Expr(Expr::Var(name))
    }

    /// Implements the `fresh_byte` symbolic ABI helper.
    pub(super) fn fresh_byte(&mut self, name: String, printable: bool) -> SymWord {
        let word = self.fresh_word(name);
        self.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ult,
            word.clone().into_expr(),
            Expr::Const(U256::from(256)),
        ));
        if printable {
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                word.clone().into_expr(),
                Expr::Const(U256::from(0x20)),
            ));
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ule,
                word.clone().into_expr(),
                Expr::Const(U256::from(0x7e)),
            ));
        }
        word
    }

    /// Implements the `next_dynamic_length` symbolic ABI helper.
    pub(super) fn next_dynamic_length(&mut self, ty: &'static str) -> Result<usize, SymbolicError> {
        let idx = self.dynamic_index;
        self.dynamic_index += 1;
        let len = self
            .config
            .array_lengths
            .get(idx)
            .copied()
            .unwrap_or(self.config.default_dynamic_length);
        if len > self.config.max_dynamic_length {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic {ty} length {len} exceeds max_dynamic_length {}",
                self.config.max_dynamic_length
            )));
        }
        Ok(len as usize)
    }

    /// Implements the `constrain_uint` symbolic ABI helper.
    pub(super) fn constrain_uint(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                word.clone().into_expr(),
                Expr::Const(U256::from(1) << bits),
            ));
        }
    }

    /// Implements the `constrain_int` symbolic ABI helper.
    pub(super) fn constrain_int(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            let byte_index = U256::from(bits / 8 - 1);
            self.constraints.push(BoolExpr::eq(
                word.clone().into_expr(),
                signextend_word(byte_index, word.clone()).into_expr(),
            ));
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum SymbolicAbiValue {
    Bool { word: SymWord },
    Uint { bits: usize, word: SymWord },
    Int { bits: usize, word: SymWord },
    FixedBytes { bytes: Vec<SymWord>, size: usize },
    Address { word: SymWord },
    Bytes { len: SymWord, bytes: Vec<SymWord> },
    String { bytes: Vec<SymWord> },
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

    /// Implements the `head_size` symbolic ABI helper.
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

    /// Implements the `encode_static` symbolic ABI helper.
    pub(super) fn encode_static(&self) -> Vec<SymWord> {
        match self {
            Self::Bool { word }
            | Self::Uint { word, .. }
            | Self::Int { word, .. }
            | Self::Address { word } => word_bytes(word.clone()),
            Self::FixedBytes { bytes, .. } => {
                let mut out = bytes.clone();
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

    /// Implements the `encode_dynamic_body` symbolic ABI helper.
    pub(super) fn encode_dynamic_body(&self) -> Vec<SymWord> {
        match self {
            Self::Bytes { len, bytes } => encode_packed_bytes_with_len(len.clone(), bytes),
            Self::String { bytes } => {
                encode_packed_bytes_with_len(SymWord::Concrete(U256::from(bytes.len())), bytes)
            }
            Self::Array { elements } => {
                let mut out = word_bytes(SymWord::Concrete(U256::from(elements.len())));
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

    /// Returns the `model_value` symbolic ABI helper result.
    pub(super) fn model_value(
        &self,
        model: &BTreeMap<String, U256>,
    ) -> Result<DynSolValue, SymbolicError> {
        Ok(match self {
            Self::Bool { word } => DynSolValue::Bool(!model_word(word, model)?.is_zero()),
            Self::Uint { bits, word } => {
                DynSolValue::Uint(mask_bits(model_word(word, model)?, *bits), *bits)
            }
            Self::Int { bits, word } => {
                DynSolValue::Int(I256::from_raw(model_word(word, model)?), *bits)
            }
            Self::FixedBytes { bytes, size } => {
                let mut word = [0u8; 32];
                for (idx, byte) in bytes.iter().enumerate() {
                    word[idx] = model_word(byte, model)?.to::<u8>();
                }
                DynSolValue::FixedBytes(B256::from(word), *size)
            }
            Self::Address { word } => {
                DynSolValue::Address(word_to_address(model_word(word, model)?))
            }
            Self::Bytes { len, bytes } => {
                let len = model_word(len, model)?;
                let len = u256_to_usize(len)
                    .filter(|len| *len <= bytes.len())
                    .ok_or_else(|| SymbolicError::Solver("invalid symbolic bytes length".into()))?;
                let mut bytes = model_bytes(bytes, model)?;
                bytes.truncate(len);
                DynSolValue::Bytes(bytes)
            }
            Self::String { bytes } => {
                let bytes = model_bytes(bytes, model)?;
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

/// Implements the `encode_sequence` symbolic ABI helper.
pub(super) fn encode_sequence<'a>(
    values: impl IntoIterator<Item = &'a SymbolicAbiValue>,
) -> Vec<SymWord> {
    let values = values.into_iter().collect::<Vec<_>>();
    let head_size = values.iter().map(|value| value.head_size()).sum::<usize>();
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();

    for value in values {
        if value.is_dynamic() {
            head.extend(word_bytes(SymWord::Concrete(U256::from(head_size + tail.len()))));
            tail.extend(value.encode_dynamic_body());
        } else {
            head.extend(value.encode_static());
        }
    }

    head.extend(tail);
    head
}

/// Implements the `encode_packed_bytes_with_len` symbolic ABI helper.
pub(super) fn encode_packed_bytes_with_len(len: SymWord, bytes: &[SymWord]) -> Vec<SymWord> {
    let mut out = word_bytes(len);
    out.extend(bytes.iter().cloned());
    out.resize(32 + bytes.len().next_multiple_of(32), SymWord::zero());
    out
}
