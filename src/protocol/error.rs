use failure::Fail;
use std::io;
use std::string::FromUtf8Error;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Incorrect packet format")]
    IncorrectPacketFormat,
    #[fail(display = "Invalid topic path")]
    InvalidTopicPath,
    #[fail(display = "Invalid topic path")]
    UnsupportedProtocolName,
    #[fail(display = "Invalid topic path")]
    UnsupportedProtocolVersion,
    #[fail(display = "Invalid topic path")]
    UnsupportedQualityOfService,
    #[fail(display = "Invalid topic path")]
    UnsupportedPacketType,
    #[fail(display = "Invalid topic path")]
    UnsupportedConnectReturnCode,
    #[fail(display = "Invalid topic path")]
    PayloadSizeIncorrect,
    #[fail(display = "Invalid topic path")]
    PayloadTooLong,
    #[fail(display = "Invalid topic path")]
    PayloadRequired,
    #[fail(display = "Invalid topic path")]
    TopicNameMustNotContainNonUtf8,
    #[fail(display = "Invalid topic path")]
    TopicNameMustNotContainWildcard,
    #[fail(display = "Invalid topic path")]
    MalformedRemainingLength,
    #[fail(display = "Invalid topic path")]
    UnexpectedEof,
    #[fail(display = "Io Error")]
    Io(#[cause] io::Error),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(_: FromUtf8Error) -> Error {
        Error::TopicNameMustNotContainNonUtf8
    }
}
