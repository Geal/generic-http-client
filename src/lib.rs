#[cfg(feature = "tls")]
extern crate rustls;
#[cfg(feature = "tls")]
extern crate webpki;
#[cfg(feature = "tls")]
extern crate webpki_roots;

pub mod accumulator;
pub mod body;
pub mod client;
pub mod error;
pub mod server;
pub mod stream;
mod util;

use error::*;

/// used to determie if the body can be sent as is or chunked
pub trait HasLength {
    fn has_length(&self) -> Option<usize>;
}

impl HasLength for Vec<u8> {
    fn has_length(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl HasLength for &[u8] {
    fn has_length(&self) -> Option<usize> {
        Some(self.len())
    }
}
