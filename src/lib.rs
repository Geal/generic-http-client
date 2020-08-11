use std::fmt::Debug;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;

pub struct Client<Stream: Read + Write, R: Resolver<Stream>> {
    //resolver: R,
    stream: Option<Stream>,
    resolver: PhantomData<R>,
}

impl<Stream: Read + Write + Debug, R: Resolver<Stream>> Client<Stream, R> {
    pub fn new(url: &str) -> Result<Self, HttpError> {
        let stream = Some(R::resolve(url)?);

        Ok(Client {
            stream,
            resolver: PhantomData,
        })
    }

    pub fn get(url_str: &str) -> Result<http::Response<Body<Stream>>, HttpError> {
        let mut client: Self = Client::new(&url_str)?;
        let mut req = http::Request::get(url_str).body(&b""[..]).unwrap();
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
        let mut stream = BufReader::new(self.stream.take().unwrap());
        let mut index = 0usize;
        let mut at_eof = false;

        loop {
            let data = stream.fill_buf()?;
            let at_eof = data.len() == 0;

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
                let mut elements = v.to_str().unwrap().split(',');

                for mut element in elements {
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

#[derive(Debug)]
pub struct Body<Stream: Read + Debug> {
    stream: BufReader<Stream>,
    length: Length,
    at_eof: bool,
}

#[derive(Debug)]
pub enum Length {
    None,
    //remaining size
    ContentLength(usize),
    // remaining size in the current chunk
    Chunked(usize),
}

impl<Stream: Read + Debug> Read for Body<Stream> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (length, res) = match self.length {
            Length::None => return Ok(0),
            Length::ContentLength(sz) => {
                if sz == 0 {
                    return Ok(0);
                }

                if self.stream.buffer().len() == 0 {
                    if self.at_eof {
                        return Ok(0);
                    } else {
                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            return Ok(0);
                        }
                    }
                }

                let bound = std::cmp::min(sz, buf.len());
                let internal_bound = std::cmp::min(bound, self.stream.buffer().len());

                let written = (&mut buf[..bound]).write(&self.stream.buffer()[..internal_bound])?;
                self.stream.consume(written);
                (Length::ContentLength(sz - written), Ok(written))
            }
            Length::Chunked(mut sz) => {
                if sz == 0 {
                    if self.stream.buffer().is_empty() {
                        if self.at_eof {
                            return Ok(0);
                        }

                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            return Ok(0);
                        }
                    }

                    let (parsed, chunk_size) = loop {
                        match httparse::parse_chunk_size(self.stream.buffer()) {
                            Err(invalid_chunk_size) => {
                                return Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    "invalid chunk size",
                                ));
                            }
                            Ok(status) => {
                                if status.is_partial() {
                                    let data = self.stream.fill_buf()?;

                                    if data.is_empty() {
                                        self.at_eof = true;
                                        return Ok(0);
                                    }
                                    continue;
                                }

                                break status.unwrap();
                            }
                        }
                    };
                    self.stream.consume(parsed);
                    sz = chunk_size as usize;
                }

                //if it is still zero, it was the last chunk
                if sz == 0 {
                    //FIXME:we might want to parse the last \r\n
                    return Ok(0);
                }

                let bound = std::cmp::min(sz, buf.len());
                // leaving two bytes to check for \r\n
                let internal_bound = std::cmp::min(bound, self.stream.buffer().len() - 2);
                let written = (&mut buf[..bound]).write(&self.stream.buffer()[..internal_bound])?;
                self.stream.consume(written);
                if sz == written {
                    if &self.stream.buffer()[..2] == &b"\r\n"[..] {
                        self.stream.consume(2);
                    } else {
                        return Err(io::Error::new(io::ErrorKind::Other, "invalid chunk end"));
                    }
                }

                (Length::ContentLength(sz - written), Ok(written))
            }
        };

        self.length = length;

        res
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

#[derive(Debug)]
pub enum ResolverError {
    NotFound,
    ConnectionFailed,
}

pub trait Resolver<Stream: Read + Write> {
    fn resolve(url: &str) -> Result<Stream, HttpError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::net::ToSocketAddrs;

    struct TcpStreamResolver {}

    impl Resolver<TcpStream> for TcpStreamResolver {
        fn resolve(url: &str) -> Result<TcpStream, HttpError> {
            let url = url::Url::parse(url).map_err(HttpError::Url)?;

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

        let mut body = res.body_mut();
        let mut s = String::new();
        body.read_to_string(&mut s).unwrap();
        println!("body:\n{}", s);
        panic!();
    }
}
