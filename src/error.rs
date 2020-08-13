use std::io;

#[derive(Debug)]
pub enum HttpError {
    Resolver(ResolverError),
    Url(url::ParseError),
    Io(io::Error),
    Parser(httparse::Error),
    Http(http::Error),
}

impl From<ResolverError> for HttpError {
    fn from(e: ResolverError) -> Self {
        HttpError::Resolver(e)
    }
}

impl From<io::Error> for HttpError {
    fn from(e: io::Error) -> Self {
        HttpError::Io(e)
    }
}

impl From<httparse::Error> for HttpError {
    fn from(e: httparse::Error) -> Self {
        HttpError::Parser(e)
    }
}

impl From<http::Error> for HttpError {
    fn from(e: http::Error) -> Self {
        HttpError::Http(e)
    }
}

impl From<url::ParseError> for HttpError {
    fn from(e: url::ParseError) -> Self {
        HttpError::Url(e)
    }
}

#[derive(Debug)]
pub enum ResolverError {
    NotFound,
    ConnectionFailed,
    InvalidScheme,
}
