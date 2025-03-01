use std::{fmt, fmt::Write, iter::Peekable, str::CharIndices};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Jump {
    /// A jump instruction that goes into a function
    In,
    /// A jump  represents an instruction that returns from a function
    Out,
    /// A regular jump instruction
    Regular,
}

impl Jump {
    /// Returns the string representation of the jump instruction.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::In => "i",
            Self::Out => "o",
            Self::Regular => "-",
        }
    }

    fn to_int(self) -> u32 {
        match self {
            Self::In => 0,
            Self::Out => 1,
            Self::Regular => 2,
        }
    }

    fn from_int(i: u32) -> Self {
        match i {
            0 => Self::In,
            1 => Self::Out,
            2 => Self::Regular,
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for Jump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_str())
    }
}

/// An error that can happen during source map parsing.
#[derive(Debug, thiserror::Error)]
pub struct SyntaxError(Box<SyntaxErrorInner>);

#[derive(Debug)]
struct SyntaxErrorInner {
    pos: Option<usize>,
    msg: String,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failed to parse source map: ")?;
        if let Some(pos) = self.0.pos {
            write!(f, "[{pos}] ")?;
        }
        f.write_str(&self.0.msg)
    }
}

impl SyntaxError {
    fn new(pos: impl Into<Option<usize>>, msg: impl Into<String>) -> Self {
        Self(Box::new(SyntaxErrorInner { pos: pos.into(), msg: msg.into() }))
    }
}

impl From<std::num::TryFromIntError> for SyntaxError {
    fn from(_value: std::num::TryFromIntError) -> Self {
        Self::new(None, "offset overflow")
    }
}

#[derive(PartialEq, Eq)]
enum Token<'a> {
    /// Decimal number
    Number(&'a str),
    /// `;`
    Semicolon,
    /// `:`
    Colon,
    /// `i` which represents an instruction that goes into a function
    In,
    /// `o` which represents an instruction that returns from a function
    Out,
    /// `-` regular jump
    Regular,
}

impl fmt::Debug for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Number(s) => write!(f, "NUMBER({s:?})"),
            Token::Semicolon => write!(f, "SEMICOLON"),
            Token::Colon => write!(f, "COLON"),
            Token::In => write!(f, "JMP(i)"),
            Token::Out => write!(f, "JMP(o)"),
            Token::Regular => write!(f, "JMP(-)"),
        }
    }
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Number(_) => write!(f, "number"),
            Token::Semicolon => write!(f, "`;`"),
            Token::Colon => write!(f, "`:`"),
            Token::In => write!(f, "jmp-in"),
            Token::Out => write!(f, "jmp-out"),
            Token::Regular => write!(f, "jmp"),
        }
    }
}

struct Lexer<'input> {
    input: &'input str,
    chars: Peekable<CharIndices<'input>>,
}

impl<'input> Lexer<'input> {
    fn new(input: &'input str) -> Self {
        Lexer { chars: input.char_indices().peekable(), input }
    }

    fn number(&mut self, start: usize, mut end: usize) -> Token<'input> {
        loop {
            if let Some((_, ch)) = self.chars.peek().cloned() {
                if !ch.is_ascii_digit() {
                    break;
                }
                self.chars.next();
                end += 1;
            } else {
                end = self.input.len();
                break;
            }
        }
        Token::Number(&self.input[start..end])
    }
}

impl<'input> Iterator for Lexer<'input> {
    type Item = Result<(Token<'input>, usize), SyntaxError>;

    fn next(&mut self) -> Option<Self::Item> {
        let (start, ch) = self.chars.next()?;
        let token = match ch {
            ';' => Token::Semicolon,
            ':' => Token::Colon,
            'i' => Token::In,
            'o' => Token::Out,
            '-' => match self.chars.peek() {
                Some((_, ch)) if ch.is_ascii_digit() => {
                    self.chars.next();
                    self.number(start, start + 2)
                }
                _ => Token::Regular,
            },
            ch if ch.is_ascii_digit() => self.number(start, start + 1),
            ch => return Some(Err(SyntaxError::new(start, format!("unexpected character: {ch}")))),
        };
        Some(Ok((token, start)))
    }
}

/// A Solidity source map, which is composed of multiple [`SourceElement`]s, separated by
/// semicolons.
///
/// Solidity reference: <https://docs.soliditylang.org/en/latest/internals/source_mappings.html#source-mappings>
pub type SourceMap = Vec<SourceElement>;

/// A single element in a [`SourceMap`].
///
/// Solidity reference: <https://docs.soliditylang.org/en/latest/internals/source_mappings.html#source-mappings>
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceElement {
    offset: u32,
    length: u32,
    index: i32,
    // 2 bits for jump, 30 bits for modifier depth; see [set_jump_and_modifier_depth]
    jump_and_modifier_depth: u32,
}

impl fmt::Debug for SourceElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceElement")
            .field("offset", &self.offset())
            .field("length", &self.length())
            .field("index", &self.index_i32())
            .field("jump", &self.jump())
            .field("modifier_depth", &self.modifier_depth())
            .field("formatted", &format_args!("{self}"))
            .finish()
    }
}

impl Default for SourceElement {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceElement {
    /// Creates a new source element with default values.
    pub fn new() -> Self {
        Self { offset: 0, length: 0, index: -1, jump_and_modifier_depth: 0 }
    }

    /// Creates a new source element with default values.
    #[deprecated = "use `new` instead"]
    pub fn new_invalid() -> Self {
        Self::new()
    }

    /// The byte-offset to the start of the range in the source file.
    #[inline]
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// The length of the source range in bytes.
    #[inline]
    pub fn length(&self) -> u32 {
        self.length
    }

    /// The source index.
    ///
    /// Note: In the case of instructions that are not associated with any particular source file,
    /// the source mapping assigns an integer identifier of -1. This may happen for bytecode
    /// sections stemming from compiler-generated inline assembly statements.
    /// This case is represented as a `None` value.
    #[inline]
    pub fn index(&self) -> Option<u32> {
        if self.index == -1 {
            None
        } else {
            Some(self.index as u32)
        }
    }

    /// The source index.
    ///
    /// See [`Self::index`] for more information.
    #[inline]
    pub fn index_i32(&self) -> i32 {
        self.index
    }

    /// Jump instruction.
    #[inline]
    pub fn jump(&self) -> Jump {
        Jump::from_int(self.jump_and_modifier_depth >> 30)
    }

    #[inline]
    fn set_jump(&mut self, jump: Jump) {
        self.set_jump_and_modifier_depth(jump, self.modifier_depth());
    }

    /// Modifier depth.
    ///
    /// This depth is increased whenever the placeholder statement (`_`) is entered in a modifier
    /// and decreased when it is left again.
    #[inline]
    pub fn modifier_depth(&self) -> u32 {
        (self.jump_and_modifier_depth << 2) >> 2
    }

    #[inline]
    fn set_modifier_depth(&mut self, modifier_depth: usize) -> Result<(), SyntaxError> {
        if modifier_depth > (1 << 30) - 1 {
            return Err(SyntaxError::new(None, "modifier depth overflow"));
        }
        self.set_jump_and_modifier_depth(self.jump(), modifier_depth as u32);
        Ok(())
    }

    #[inline]
    fn set_jump_and_modifier_depth(&mut self, jump: Jump, modifier_depth: u32) {
        self.jump_and_modifier_depth = (jump.to_int() << 30) | modifier_depth;
    }
}

impl fmt::Display for SourceElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}:{}:{}",
            self.offset(),
            self.length(),
            self.index_i32(),
            self.jump(),
            self.modifier_depth(),
        )
    }
}

#[derive(Default)]
struct SourceElementBuilder {
    offset: Option<usize>,
    length: Option<usize>,
    index: Option<Option<u32>>,
    jump: Option<Jump>,
    modifier_depth: Option<usize>,
}

impl fmt::Display for SourceElementBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.offset.is_none()
            && self.length.is_none()
            && self.index.is_none()
            && self.jump.is_none()
            && self.modifier_depth.is_none()
        {
            return Ok(());
        }

        if let Some(s) = self.offset {
            if s == 0 && self.index == Some(None) {
                f.write_str("-1")?;
            } else {
                write!(f, "{s}")?;
            }
        }
        if self.length.is_none()
            && self.index.is_none()
            && self.jump.is_none()
            && self.modifier_depth.is_none()
        {
            return Ok(());
        }
        f.write_char(':')?;

        if let Some(s) = self.length {
            if s == 0 && self.index == Some(None) {
                f.write_str("-1")?;
            } else {
                write!(f, "{s}")?;
            }
        }
        if self.index.is_none() && self.jump.is_none() && self.modifier_depth.is_none() {
            return Ok(());
        }
        f.write_char(':')?;

        if let Some(s) = self.index {
            let s = s.map(|s| s as i64).unwrap_or(-1);
            write!(f, "{s}")?;
        }
        if self.jump.is_none() && self.modifier_depth.is_none() {
            return Ok(());
        }
        f.write_char(':')?;

        if let Some(s) = self.jump {
            write!(f, "{s}")?;
        }
        if self.modifier_depth.is_none() {
            return Ok(());
        }
        f.write_char(':')?;

        if let Some(s) = self.modifier_depth {
            if self.index == Some(None) {
                f.write_str("-1")?;
            } else {
                s.fmt(f)?;
            }
        }

        Ok(())
    }
}

impl SourceElementBuilder {
    fn finish(self, prev: Option<SourceElement>) -> Result<SourceElement, SyntaxError> {
        let mut element = prev.unwrap_or_default();
        macro_rules! get_field {
            (| $field:ident | $e:expr) => {
                if let Some($field) = self.$field {
                    $e;
                }
            };
        }
        get_field!(|offset| element.offset = offset.try_into()?);
        get_field!(|length| element.length = length.try_into()?);
        get_field!(|index| element.index = index.map(|x| x as i32).unwrap_or(-1));
        get_field!(|jump| element.set_jump(jump));
        // Modifier depth is optional.
        if let Some(modifier_depth) = self.modifier_depth {
            element.set_modifier_depth(modifier_depth)?;
        }
        Ok(element)
    }

    fn set_jmp(&mut self, jmp: Jump, pos: usize) -> Result<(), SyntaxError> {
        if self.jump.is_some() {
            return Err(SyntaxError::new(pos, "jump already set"));
        }
        self.jump = Some(jmp);
        Ok(())
    }

    fn set_offset(&mut self, offset: usize, pos: usize) -> Result<(), SyntaxError> {
        if self.offset.is_some() {
            return Err(SyntaxError::new(pos, "offset already set"));
        }
        self.offset = Some(offset);
        Ok(())
    }

    fn set_length(&mut self, length: usize, pos: usize) -> Result<(), SyntaxError> {
        if self.length.is_some() {
            return Err(SyntaxError::new(pos, "length already set"));
        }
        self.length = Some(length);
        Ok(())
    }

    fn set_index(&mut self, index: Option<u32>, pos: usize) -> Result<(), SyntaxError> {
        if self.index.is_some() {
            return Err(SyntaxError::new(pos, "index already set"));
        }
        self.index = Some(index);
        Ok(())
    }

    fn set_modifier(&mut self, modifier_depth: usize, pos: usize) -> Result<(), SyntaxError> {
        if self.modifier_depth.is_some() {
            return Err(SyntaxError::new(pos, "modifier depth already set"));
        }
        self.modifier_depth = Some(modifier_depth);
        Ok(())
    }
}

pub struct Parser<'input> {
    lexer: Lexer<'input>,
    last_element: Option<SourceElement>,
    done: bool,
    #[cfg(test)]
    output: Option<&'input mut dyn Write>,
}

impl<'input> Parser<'input> {
    pub fn new(input: &'input str) -> Self {
        Self {
            done: input.is_empty(),
            lexer: Lexer::new(input),
            last_element: None,
            #[cfg(test)]
            output: None,
        }
    }

    fn advance(&mut self) -> Result<Option<SourceElement>, SyntaxError> {
        // start parsing at the offset state, `s`
        let mut state = State::Offset;
        let mut builder = SourceElementBuilder::default();

        let parse_number = |num: &str, pos: usize| {
            let num = match num.parse::<i64>() {
                Ok(num) => num,
                Err(e) => return Err(SyntaxError::new(pos, e.to_string())),
            };
            match num {
                ..-1 => Err(SyntaxError::new(pos, "unexpected negative number")),
                -1 => Ok(None),
                0.. => u32::try_from(num)
                    .map(Some)
                    .map_err(|_| SyntaxError::new(pos, "number too large")),
            }
        };

        loop {
            match self.lexer.next() {
                Some(Ok((token, pos))) => match token {
                    Token::Semicolon => break,
                    Token::Number(num) => match state {
                        State::Offset => {
                            builder
                                .set_offset(parse_number(num, pos)?.unwrap_or(0) as usize, pos)?;
                        }
                        State::Length => {
                            builder
                                .set_length(parse_number(num, pos)?.unwrap_or(0) as usize, pos)?;
                        }
                        State::Index => {
                            builder.set_index(parse_number(num, pos)?, pos)?;
                        }
                        State::Modifier => builder
                            .set_modifier(parse_number(num, pos)?.unwrap_or(0) as usize, pos)?,
                        State::Jmp => {
                            return Err(SyntaxError::new(pos, "expected jump, found number"));
                        }
                    },
                    Token::Colon => state.advance(pos)?,
                    Token::In => builder.set_jmp(Jump::In, pos)?,
                    Token::Out => builder.set_jmp(Jump::Out, pos)?,
                    Token::Regular => builder.set_jmp(Jump::Regular, pos)?,
                },
                Some(Err(err)) => return Err(err),
                None => {
                    if self.done {
                        return Ok(None);
                    }
                    self.done = true;
                    break;
                }
            }
        }

        #[cfg(test)]
        if let Some(out) = self.output.as_mut() {
            if self.last_element.is_some() {
                out.write_char(';').unwrap();
            }
            write!(out, "{builder}").unwrap();
        }

        let element = builder.finish(self.last_element.take())?;
        self.last_element = Some(element.clone());
        Ok(Some(element))
    }
}

impl Iterator for Parser<'_> {
    type Item = Result<SourceElement, SyntaxError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance().transpose()
    }
}

/// State machine to keep track of separating `:`
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    // s
    Offset,
    // l
    Length,
    // f
    Index,
    // j
    Jmp,
    // m
    Modifier,
}

impl State {
    fn advance(&mut self, pos: usize) -> Result<(), SyntaxError> {
        *self = match self {
            Self::Offset => Self::Length,
            Self::Length => Self::Index,
            Self::Index => Self::Jmp,
            Self::Jmp => Self::Modifier,
            Self::Modifier => return Err(SyntaxError::new(pos, "unexpected colon")),
        };
        Ok(())
    }
}

/// Parses a source map.
pub fn parse(input: &str) -> Result<SourceMap, SyntaxError> {
    Parser::new(input).collect::<Result<SourceMap, SyntaxError>>().map(|mut v| {
        v.shrink_to_fit();
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test(input: &str) {
        match parse_test_(input) {
            Ok(_) => {}
            Err(e) => panic!("{e}"),
        }
    }

    fn parse_test_(input: &str) -> Result<SourceMap, SyntaxError> {
        let mut s = String::new();
        let mut p = Parser::new(input);
        p.output = Some(&mut s);
        let sm = p.collect::<Result<SourceMap, _>>()?;
        if s != input {
            return Err(SyntaxError::new(
                None,
                format!("mismatched output:\n   actual: {s:?}\n expected: {input:?}\n       sm: {sm:#?}"),
            ));
        }
        Ok(sm)
    }

    #[test]
    fn empty() {
        parse_test("");
    }

    #[test]
    fn source_maps() {
        // all source maps from the compiler output test data
        let source_maps = include_str!("../../../../test-data/out-source-maps.txt");

        for (line, s) in source_maps.lines().enumerate() {
            let line = line + 1;
            parse_test_(s).unwrap_or_else(|e| panic!("Failed to parse line {line}: {e}\n{s:?}"));
        }
    }

    #[test]
    fn cheatcodes() {
        let s = include_str!("../../../../test-data/cheatcodes.sol-sourcemap.txt");
        parse_test(s);
    }

    // https://github.com/foundry-rs/foundry/issues/8986
    #[test]
    fn univ4_deployer() {
        let s = ":::-:0;;1888:10801:91;2615:100;;;2679:3;2615:100;;;;2700:4;2615:100;;;;-1:-1:-1;2615:100:91;;;;2546:169;;;-1:-1:-1;;2546:169:91;;;;;;;;;;;2615:100;2546:169;;;2615:100;2797:101;;;;;;;;;-1:-1:-1;;2797:101:91;;;;;;;;2546:169;2721:177;;;;;;;;;;;;;;;;;;2957:101;1888:10801;2957:101;2797;2957;;;-1:-1:-1;;2957:101:91;;;;356:29:89;2957:101:91;;;;2904:154;;;-1:-1:-1;;2904:154:91;;;;;;;;;;;;-1:-1:-1;;;;;;2904:154:91;;;;;;;;4018:32;;;;;4048:2;4018:32;;;4056:74;;;-1:-1:-1;;;;;4056:74:91;;;;;;;;1888:10801;;;;;;;;;;;;;;;;";
        parse_test(s);
    }
}
