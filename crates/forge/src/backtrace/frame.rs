//! Stack frame representation for backtraces.

use alloy_primitives::Address;
use std::fmt;

/// A single frame in a backtrace.
#[derive(Debug, Clone)]
pub struct BacktraceFrame {
    /// The contract address where this frame is executing.
    pub contract_address: Address,
    /// The contract name, if known.
    pub contract_name: Option<String>,
    /// The function name, if known.
    pub function_name: Option<String>,
    /// The source file path.
    pub file: Option<String>,
    /// The line number in the source file.
    pub line: Option<usize>,
    /// The column number in the source file.
    pub column: Option<usize>,
    /// The kind of frame.
    pub kind: BacktraceFrameKind,
}

/// The kind of frame in a backtrace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BacktraceFrameKind {
    /// A user-defined function.
    UserFunction,
    /// A test function.
    TestFunction,
    /// A library function.
    LibraryFunction,
    /// An external contract call.
    ExternalCall,
    /// A fallback function.
    Fallback,
    /// A receive function.
    Receive,
    /// A constructor.
    Constructor,
    /// Internal/compiler-generated code.
    Internal,
}

impl BacktraceFrame {
    /// Creates a new backtrace frame.
    pub fn new(contract_address: Address) -> Self {
        Self {
            contract_address,
            contract_name: None,
            function_name: None,
            file: None,
            line: None,
            column: None,
            kind: BacktraceFrameKind::UserFunction,
        }
    }

    /// Sets the contract name.
    pub fn with_contract_name(mut self, name: String) -> Self {
        self.contract_name = Some(name);
        self
    }

    /// Sets the function name.
    pub fn with_function_name(mut self, name: String) -> Self {
        self.function_name = Some(name);
        self
    }

    /// Sets the source location.
    pub fn with_source_location(mut self, file: String, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Sets the frame kind.
    pub fn with_kind(mut self, kind: BacktraceFrameKind) -> Self {
        self.kind = kind;
        self
    }

    /// Returns a formatted string for this frame.
    pub fn format(&self) -> String {
        let mut result = String::new();

        // Format: file:line:column or just ContractName if no file info
        if let Some(ref file) = self.file {
            // Start with file path
            result.push_str(file);
            
            // Add line and column directly after file path
            if let Some(line) = self.line {
                result.push(':');
                result.push_str(&line.to_string());
                if let Some(column) = self.column {
                    result.push(':');
                    result.push_str(&column.to_string());
                }
            }
        } else {
            // No file info - try to show at least something useful
            // Format: ContractName or address if no name available
            if let Some(ref contract) = self.contract_name {
                // Try to infer file path from contract name
                if contract.contains(':') {
                    // Already has file path like "src/SomeFile.sol:ContractName"
                    result.push_str(contract);
                } else {
                    // Just contract name - we don't know the file path
                    // Show as <ContractName> to indicate missing file info
                    result.push_str("<");
                    result.push_str(contract);
                    result.push_str(">");
                }
            } else {
                // No contract name, show address
                result.push_str(&format!("<Contract {}>", self.contract_address));
            }
            
            // Add function if available
            if let Some(ref func) = self.function_name {
                result.push('.');
                result.push_str(func);
                result.push_str("()");
            } else {
                match self.kind {
                    BacktraceFrameKind::Fallback => result.push_str(".<fallback>()"),
                    BacktraceFrameKind::Receive => result.push_str(".<receive>()"),
                    BacktraceFrameKind::Constructor => result.push_str(".<constructor>()"),
                    _ => {}
                }
            }
            
            // Only add line:column if we have at least line info
            if self.line.is_some() {
                if let Some(line) = self.line {
                    result.push(':');
                    result.push_str(&line.to_string());
                    if let Some(column) = self.column {
                        result.push(':');
                        result.push_str(&column.to_string());
                    } else {
                        result.push_str(":0");
                    }
                }
            }
        }

        result
    }
}

impl fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}