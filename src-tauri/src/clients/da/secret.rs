use std::fmt;

/// A value that may cross only trusted Rust boundaries.
///
/// It deliberately implements neither `Display` nor `Serialize`; `Debug`
/// never delegates to the wrapped value.
pub(crate) struct Secret<T>(T);

impl<T> Secret<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }

    pub(crate) fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}
