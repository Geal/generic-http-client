use std::fmt::Debug;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;

pub mod body;
pub mod error;
pub mod stream;
mod accumulator;

use body::*;
use error::*;
use stream::*;

pub struct Client<Stream: Read + Write, R: Resolver<Stream>> {
    //resolver: R,
    stream: Option<HttpStream<Stream>>,
    resolver: PhantomData<R>,
}

impl<Stream: Read + Write + Debug, R: Resolver<Stream>> Client<Stream, R> {
    pub fn new(url: &str) -> Result<Self, HttpError> {
        let url = url::Url::parse(url).map_err(HttpError::Url)?;
        let stream = R::resolve(url.clone())?;

        let stream = Some(match url.scheme() {
            "http" => HttpStream::plaintext(stream),
            "https" => HttpStream::tls(stream, url.host_str().unwrap()),
            _ => return Err(ResolverError::InvalidScheme.into()),
        });

        Ok(Client {
            stream,
            resolver: PhantomData,
        })
    }

    pub fn get(url_str: &str) -> Result<http::Response<Body<Stream>>, HttpError> {
        let mut client: Self = Client::new(&url_str)?;
        let mut req: http::Request::<&'static [u8]> = http::Request::default();
        *req.method_mut() = http::Method::GET;
        let url = url::Url::parse(url_str).map_err(HttpError::Url)?;
        req.headers_mut().insert(http::header::HOST, http::header::HeaderValue::from_str(url.host_str().unwrap()).unwrap());
        *req.uri_mut() = url[url::Position::BeforePath..].parse().unwrap();

        client.send(req)?;
        let response = client.receive()?;
        Ok(response)
    }

    pub fn send<T: BufRead + HasLength>(
        &mut self,
        mut req: http::Request<T>,
    ) -> Result<(), HttpError> {
        let mut stream = BufWriter::new(self.stream.take().unwrap());

        // we'are assuming that the reqest line and all headers will fit into the buffer
        write!(
            &mut stream,
            "{} {} HTTP/1.1\r\n",
            req.method().as_str(),
            req.uri().to_string()
        )?;

        for (name, value) in req.headers() {
            write!(&mut stream, "{}: ", name.as_str())?;
            stream.write_all(value.as_ref())?;
            stream.write_all(&b"\r\n"[..])?;
        }

        if let Some(sz) = req.body().has_length() {
            write!(&mut stream, "Content-Length: {}\r\n", sz)?;
        } else {
            stream.write_all(&b"Transfer-Encoding: Chunked\r\n"[..])?;
        }

        let has_length = req.body().has_length().is_some();
        stream.write_all(&b"\r\n"[..])?;

        if has_length {
            std::io::copy(req.body_mut(), &mut stream)?;
        } else {
            loop {
                let data = req.body_mut().fill_buf()?;
                if data.len() == 0 {
                    //EOF
                    stream.write_all(&b"0\r\n\r\n"[..])?;
                    break;
                } else {
                    write!(&mut stream, "{:x?}\r\n", data.len())?;
                    stream.write_all(data)?;
                    stream.write_all(&b"\r\n"[..])?;
                }
            }
        }
        stream.flush()?;

        let stream = match stream.into_inner() {
            Ok(s) => s,
            Err(_) => panic!(),
        };

        self.stream = Some(stream);

        Ok(())
    }

    fn receive(&mut self) -> Result<http::Response<Body<Stream>>, HttpError> {
        let mut response = http::Response::builder();
        let mut stream = accumulator::AccReader::with_capacity(16384, self.stream.take().unwrap());
        let mut at_eof;

        loop {
            let data = stream.fill_buf()?;
            at_eof = data.len() == 0;

            let mut headers = [httparse::EMPTY_HEADER; 30];
            let mut res = httparse::Response::new(&mut headers);

            let status = res.parse(stream.buffer())?;
            if status.is_partial() {
                if at_eof {
                    panic!("got partial response and EOF");
                } else {
                    continue;
                }
            }

            let parsed_length = status.unwrap();
            response = response.status(res.code.unwrap());
            //    .version(res.version.unwrap());

            for header in res.headers {
                response = response.header(header.name, std::str::from_utf8(header.value).unwrap());
            }

            stream.consume(parsed_length);
            break;
        }

        let mut length = Length::None;
        if let Some(headers) = response.headers_ref() {
            if let Some(v) = headers.get(http::header::CONTENT_LENGTH) {
                if let Ok(nb) = v.to_str().unwrap().parse::<usize>() {
                    length = Length::ContentLength(nb);
                }
            }

            if let Some(v) = headers.get(http::header::TRANSFER_ENCODING) {
                let elements = v.to_str().unwrap().split(',');

                for element in elements {
                    let s = element.trim().to_lowercase();
                    if &s == "chunked" {
                        length = Length::Chunked(0);
                    }
                }
            }
        }

        let body = Body {
            stream,
            length,
            at_eof,
        };

        Ok(response.body(body)?)
    }
}

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

pub trait Resolver<Stream: Read + Write> {
    fn resolve(url: url::Url) -> Result<Stream, HttpError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::net::ToSocketAddrs;

    struct TcpStreamResolver {}

    impl Resolver<TcpStream> for TcpStreamResolver {
        fn resolve(url: url::Url) -> Result<TcpStream, HttpError> {
            let host = match url.port_or_known_default() {
                Some(p) => format!("{}:{}", url.host_str().unwrap(), p),
                None => url.host_str().unwrap().to_string(),
            };
            println!("resolving hostname: {}", host);
            match host.to_socket_addrs() {
                Err(e) => {
                    println!("ToSocketAddrs error: {:?}", e);
                    Err(ResolverError::NotFound.into())
                }
                Ok(mut addr_iter) => match addr_iter.next() {
                    None => {
                        println!("ToSocketAddrs error: no addresses returned");
                        Err(ResolverError::NotFound.into())
                    }
                    Some(addr) => match TcpStream::connect(addr) {
                        Err(_) => Err(ResolverError::ConnectionFailed.into()),
                        Ok(stream) => Ok(stream),
                    },
                },
            }
        }
    }

    #[test]
    fn clever_cloud() {
        let mut res =
            Client::<TcpStream, TcpStreamResolver>::get("http://www.clever-cloud.com/").unwrap();

        println!("got response:\n{:?}", res);

        let body = res.body_mut();
        let mut s = String::new();
        body.read_to_string(&mut s).unwrap();
        println!("body:\n{}", s);
        panic!();
    }
}
