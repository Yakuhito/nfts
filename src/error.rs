use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CliError {
    Message(String),
    Sqlx(sqlx::Error),
    Io(std::io::Error),
    Bech32(bech32::DecodeError),
    Bech32Encode(bech32::EncodeError),
    Hrp(bech32::primitives::hrp::Error),
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
            Self::Sqlx(err) => write!(f, "database error: {err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Bech32(err) => write!(f, "bech32 decode error: {err}"),
            Self::Bech32Encode(err) => write!(f, "bech32 encode error: {err}"),
            Self::Hrp(err) => write!(f, "bech32 hrp error: {err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<sqlx::Error> for CliError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sqlx(value)
    }
}

impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<bech32::DecodeError> for CliError {
    fn from(value: bech32::DecodeError) -> Self {
        Self::Bech32(value)
    }
}

impl From<bech32::EncodeError> for CliError {
    fn from(value: bech32::EncodeError) -> Self {
        Self::Bech32Encode(value)
    }
}

impl From<bech32::primitives::hrp::Error> for CliError {
    fn from(value: bech32::primitives::hrp::Error) -> Self {
        Self::Hrp(value)
    }
}
