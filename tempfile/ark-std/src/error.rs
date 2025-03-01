use crate::boxed::Box;
use crate::fmt::{self, Debug, Display};
use crate::string::String;

pub trait Error: core::fmt::Debug + core::fmt::Display {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

impl<'a, E: Error + 'a> From<E> for Box<dyn Error + 'a> {
    fn from(err: E) -> Self {
        Box::new(err)
    }
}

impl<'a, E: Error + Send + Sync + 'a> From<E> for Box<dyn Error + Send + Sync + 'a> {
    fn from(err: E) -> Box<dyn Error + Send + Sync + 'a> {
        Box::new(err)
    }
}

impl<T: Error> Error for Box<T> {}

impl From<String> for Box<dyn Error + Send + Sync> {
    #[inline]
    fn from(err: String) -> Box<dyn Error + Send + Sync> {
        struct StringError(String);

        impl Error for StringError {}

        impl Display for StringError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                Display::fmt(&self.0, f)
            }
        }

        // Purposefully skip printing "StringError(..)"
        impl Debug for StringError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                Debug::fmt(&self.0, f)
            }
        }

        Box::new(StringError(err))
    }
}

impl<'a> From<&'a str> for Box<dyn Error + Send + Sync> {
    #[inline]
    fn from(err: &'a str) -> Box<dyn Error + Send + Sync> {
        From::from(String::from(err))
    }
}
