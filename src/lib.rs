use std::io::{self, Read, Write, BufWriter, BufRead, ErrorKind};
use std::fmt::Debug;
use std::marker::PhantomData;

pub struct Client<Stream: Read+Write, R: Resolver<Stream>> {
    //resolver: R,
    stream: Option<Stream>,
    resolver: PhantomData<R>,
}

impl<Stream: Read+Write+Debug, R: Resolver<Stream>> Client<Stream, R> {
    pub fn new(hostname: &str) -> Result<Self, HttpError> {
        let stream = Some(R::resolve(hostname).map_err(HttpError::Resolver)?);

        Ok(Client { stream, resolver: PhantomData })
    }

    pub fn get(url_str: &str) -> Result<http::Response<Body<Stream>>, HttpError> {
        let url = url::Url::parse(url_str).map_err(HttpError::Url)?;

        let host = match url.port() {
            Some(p) => format!("{}:{}", url.host_str().unwrap(), p),
            None => url.host_str().unwrap().to_string(),
        };

        let mut client: Self = Client::new(&host)?;
        let mut req = http::Request::get(url_str).body(&b""[..]).unwrap();
        client.send(req)?;
        let response = client.receive()?;
        Ok(response)
    }

    pub fn send<T: BufRead+HasLength>(&mut self, mut req: http::Request<T>) -> Result<(), HttpError> {
        let mut stream = BufWriter::new(self.stream.take().unwrap());

        // we'are assuming that the reqest line and all headers will fit into the buffer
        write!(&mut stream, "{} {} HTTP/1.1\r\n", req.method().as_str(), req.uri().to_string())?;

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
        let mut buffer: Vec<u8> = std::iter::repeat(0).take(16384).collect();

        let mut response = http::Response::builder();
        let mut stream = self.stream.take().unwrap();
        let mut index = 0usize;
        let mut at_eof = false;
        let parsed_length = loop {
            if index == buffer.len() {
                panic!("the buffer was too small to parse the response");
            }

            let sz = stream.read(&mut buffer[index..])?;
            index += sz;

            let at_eof = sz == 0;


            let mut headers = [httparse::EMPTY_HEADER; 30];
            let mut res = httparse::Response::new(&mut headers);

            let status = res.parse(&buffer[..index])?;
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

            break parsed_length;
        };

        buffer = buffer.split_off(parsed_length);
        println!("parsed {} bytes, got response:\n{:?}", parsed_length, response);

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
                        length = Length::Chunked;
                    }
                }

            }

        }

        let body = Body {
          stream,
          length,
          buffer,
          at_eof,
        };

        Ok(response.body(body)?)
    }
}

#[derive(Debug)]
pub struct Body<Stream: Read+Debug> {
    stream: Stream,
    length: Length,
    buffer: Vec<u8>,
    at_eof: bool,
}

#[derive(Debug)]
pub enum Length {
    None,
    ContentLength(usize),
    Chunked,
}

impl<Stream: Read+Debug> Body {
    fn fill(&mut self) -> io::Result<usize> {


    }
}


/*
impl<Stream: Read+Debug> Read for Body<Stream> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (length, res) = match self.length {
            Length::None => return Ok(0),
            Length::ContentLength(sz) => {
                let bound = std::cmp::min(sz, buf.len());

                let mut index = 0usize;

                loop {




                }



            },
            Length::Chunked => {

            },
        };

        self.length = length;

        res
    }
}
*/

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

pub trait Resolver<Stream: Read+Write> {
    fn resolve(hostname: &str) -> Result<Stream, ResolverError>;
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::net::ToSocketAddrs;

    struct TcpStreamResolver {}

    impl Resolver<TcpStream> for TcpStreamResolver {
        fn resolve(hostname: &str) -> Result<TcpStream, ResolverError> {
            println!("resolving hostname: {}", hostname);
            match hostname.to_socket_addrs() {
                Err(e) => {
                    println!("ToSocketAddrs error: {:?}", e);
                    Err(ResolverError::NotFound)
                },
                Ok(mut addr_iter) => match addr_iter.next() {
                    None => {
                        println!("ToSocketAddrs error: no addresses returned");
                        Err(ResolverError::NotFound)
                    },
                    Some(addr) => match TcpStream::connect(addr) {
                        Err(_) => Err(ResolverError::ConnectionFailed),
                        Ok(stream) => Ok(stream)
                    }
                }
            }
        }
    }

    #[test]
    fn localhost() {
        let res = Client::<TcpStream, TcpStreamResolver>::get("http://lolcatho.st:1026/").unwrap();

        println!("got response:\n{:?}", res);
        panic!();

    }
}
