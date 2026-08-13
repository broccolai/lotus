use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub struct NativeError(windows::core::Error);

impl Display for NativeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl Error for NativeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<windows::core::Error> for NativeError {
    fn from(error: windows::core::Error) -> Self {
        Self(error)
    }
}
