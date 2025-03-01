use core::{
    fmt::{self, Write},
    ops,
};

/// Enum to represent the sign of a 256-bit signed integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i8)]
pub enum Sign {
    /// Less than zero.
    Negative = -1,
    /// Greater than or equal to zero.
    Positive = 1,
}

impl ops::Mul for Sign {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Positive, Self::Positive) => Self::Positive,
            (Self::Positive, Self::Negative) => Self::Negative,
            (Self::Negative, Self::Positive) => Self::Negative,
            (Self::Negative, Self::Negative) => Self::Positive,
        }
    }
}

impl ops::Neg for Sign {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }
}

impl ops::Not for Sign {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }
}

impl fmt::Display for Sign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self, f.sign_plus()) {
            (Self::Positive, false) => Ok(()),
            _ => f.write_char(self.as_char()),
        }
    }
}

impl Sign {
    /// Equality at compile-time.
    #[inline]
    pub const fn const_eq(self, other: Self) -> bool {
        self as i8 == other as i8
    }

    /// Returns whether the sign is positive.
    #[inline]
    pub const fn is_positive(&self) -> bool {
        matches!(self, Self::Positive)
    }

    /// Returns whether the sign is negative.
    #[inline]
    pub const fn is_negative(&self) -> bool {
        matches!(self, Self::Negative)
    }

    /// Returns the sign character.
    #[inline]
    pub const fn as_char(&self) -> char {
        match self {
            Self::Positive => '+',
            Self::Negative => '-',
        }
    }
}
