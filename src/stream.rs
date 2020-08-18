#[cfg(feature = "tls")]
use rustls::{ClientConfig, ClientSession, StreamOwned};
use std::io::{self, Read, Write};
use std::sync::Arc;

pub enum HttpStream<Stream: Read + Write> {
    Plain(Stream),
    #[cfg(feature = "tls")]
    Tls(StreamOwned<ClientSession, Stream>),
}

impl<Stream: Read + Write> HttpStream<Stream> {
    pub fn plaintext(stream: Stream) -> HttpStream<Stream> {
        HttpStream::Plain(stream)
    }

    #[cfg(feature = "tls")]
    pub fn tls(stream: Stream, host: &str) -> HttpStream<Stream> {
        let mut config = ClientConfig::new();
        config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);

        let dns_name = webpki::DNSNameRef::try_from_ascii_str(host).unwrap();
        let sess = ClientSession::new(&Arc::new(config), dns_name);
        HttpStream::Tls(StreamOwned::new(sess, stream))
    }

    #[cfg(not(feature = "tls"))]
    pub fn tls(stream: Stream, host: &str) -> HttpStream<Stream> {
        unimplemented!()
    }
}

impl<Stream: Read + Write> Read for HttpStream<Stream> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            HttpStream::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            HttpStream::Tls(s) => s.read(buf),
        }
    }
}

impl<Stream: Read + Write> Write for HttpStream<Stream> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            HttpStream::Plain(s) => s.write(buf),
            #[cfg(feature = "tls")]
            HttpStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            HttpStream::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            HttpStream::Tls(s) => s.flush(),
        }
    }
}

impl<Stream: Read + Write> std::fmt::Debug for HttpStream<Stream> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("HttpStream").finish()
    }
}
