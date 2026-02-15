use chia_wallet_sdk::{driver::DriverError, utils::Bech32Error};
use reqwest::Error as RequestError;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CliError {
    Message(String),
    Sqlx(sqlx::Error),
    Io(std::io::Error),
    Bech32(Bech32Error),
    Driver(DriverError),
    Request(RequestError),
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
            Self::Sqlx(err) => write!(f, "database error: {err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Bech32(err) => write!(f, "{err}"),
            // Transparent-like formatting: keep upstream DriverError wording unchanged.
            Self::Driver(err) => write!(f, "{err}"),
            Self::Request(err) => write!(f, "{err}"),
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

impl From<Bech32Error> for CliError {
    fn from(value: Bech32Error) -> Self {
        Self::Bech32(value)
    }
}

impl From<DriverError> for CliError {
    fn from(value: DriverError) -> Self {
        Self::Driver(value)
    }
}

impl From<reqwest::Error> for CliError {
    fn from(value: reqwest::Error) -> Self {
        Self::Request(value)
    }
}
