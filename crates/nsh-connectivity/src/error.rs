//! The errors a connectivity call reports. Each has a stable code; a message says what happened.

use std::fmt;

/// What went wrong, as a stable code; see [`ConnectivityError::code`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The platform has no connectivity API here.
    NotAvailable,
    /// The OS refused the query.
    PermissionDenied,
    /// The caller cancelled: a dropped future, `stop`, or the platform.
    Cancelled,
    /// The operation gave up at its deadline.
    Timeout,
    /// `fetch` named an interface the platform does not have.
    UnknownInterface,
    /// The platform's own service failed; `status` holds its code where it gives one.
    Backend,
    /// An argument is empty or not allowed.
    InvalidArgument,
}

impl ErrorKind {
    /// The stable code of this kind.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::NotAvailable => "ERR_CONNECTIVITY_NOT_AVAILABLE",
            ErrorKind::PermissionDenied => "ERR_CONNECTIVITY_PERMISSION_DENIED",
            ErrorKind::Cancelled => "ERR_CONNECTIVITY_CANCELLED",
            ErrorKind::Timeout => "ERR_CONNECTIVITY_TIMEOUT",
            ErrorKind::UnknownInterface => "ERR_CONNECTIVITY_UNKNOWN_INTERFACE",
            ErrorKind::Backend => "ERR_CONNECTIVITY_BACKEND",
            ErrorKind::InvalidArgument => "ERR_CONNECTIVITY_INVALID_ARGUMENT",
        }
    }

    /// What happened, in one sentence; the TS package uses the same wording.
    pub fn message(self) -> &'static str {
        match self {
            ErrorKind::NotAvailable => {
                "The platform has no connectivity API here (an unsupported target, or the web lacks the API)."
            }
            ErrorKind::PermissionDenied => {
                "The OS refused the query, such as Android's missing location permission for SSID."
            }
            ErrorKind::Cancelled => {
                "The caller cancelled: dropping the future, calling stop, or a cancel from the platform."
            }
            ErrorKind::Timeout => "The operation gave up at its deadline instead of hanging.",
            ErrorKind::UnknownInterface => "fetch() named an interface the platform does not have.",
            ErrorKind::Backend => {
                "The platform's own service failed; the error carries the platform status where it gives one."
            }
            ErrorKind::InvalidArgument => "An argument is empty or not allowed.",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A failed connectivity call. `Display` writes the stable code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .kind.code())]
pub struct ConnectivityError {
    kind: ErrorKind,
    status: Option<i64>,
}

impl ConnectivityError {
    /// An error of `kind`, with no platform status.
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, status: None }
    }

    /// The same error, carrying the platform's own status (an HRESULT, an errno).
    #[must_use]
    pub fn with_status(mut self, status: i64) -> Self {
        self.status = Some(status);
        self
    }

    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The platform's status, when it gave one.
    pub fn status(&self) -> Option<i64> {
        self.status
    }

    /// The stable code, such as `ERR_CONNECTIVITY_CANCELLED`.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// What happened, in one sentence.
    pub fn message(&self) -> &'static str {
        self.kind.message()
    }
}

impl From<ErrorKind> for ConnectivityError {
    fn from(kind: ErrorKind) -> Self {
        Self::new(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_code_and_a_message() {
        let kinds = [
            (ErrorKind::NotAvailable, "ERR_CONNECTIVITY_NOT_AVAILABLE"),
            (
                ErrorKind::PermissionDenied,
                "ERR_CONNECTIVITY_PERMISSION_DENIED",
            ),
            (ErrorKind::Cancelled, "ERR_CONNECTIVITY_CANCELLED"),
            (ErrorKind::Timeout, "ERR_CONNECTIVITY_TIMEOUT"),
            (
                ErrorKind::UnknownInterface,
                "ERR_CONNECTIVITY_UNKNOWN_INTERFACE",
            ),
            (ErrorKind::Backend, "ERR_CONNECTIVITY_BACKEND"),
            (
                ErrorKind::InvalidArgument,
                "ERR_CONNECTIVITY_INVALID_ARGUMENT",
            ),
        ];
        for (kind, code) in kinds {
            let error = ConnectivityError::new(kind);
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), code);
            assert!(!error.message().is_empty());
            assert_eq!(error.status(), None);
        }
        assert_eq!(
            ConnectivityError::new(ErrorKind::Backend)
                .with_status(-5)
                .status(),
            Some(-5)
        );
    }
}
