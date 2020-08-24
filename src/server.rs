use crate::accumulator::AccReader;
use crate::body::{Body, Length};
use crate::util;
use crate::HasLength;
use crate::HttpError;
use std::fmt::Debug;
use std::io::{BufRead, BufWriter, Read, Write};

pub struct Server;

pub fn parse<Stream: Read + Write + Debug>(
    stream: Stream,
) -> Result<http::Request<Body<Stream>>, HttpError> {
    let mut request = http::Request::builder();
    let mut stream = AccReader::with_capacity(16384, stream);
    let mut at_eof;

    loop {
        //println!("loop: bufferlen == {}", stream.buffer().len());
        let data = stream.fill_buf()?;
        at_eof = data.len() == 0;

        let mut headers = [httparse::EMPTY_HEADER; 30];
        let mut req = httparse::Request::new(&mut headers);

        /*println!(
            "will parse:\nraw: {:x?}\n{}",
            stream.buffer(),
            std::str::from_utf8(stream.buffer()).unwrap()
        );
        */
        let status = req.parse(stream.buffer())?;
        //println!("parser result: {:?}\n{:?}", status, req);
        if status.is_partial() {
            if at_eof {
                panic!("got partial response and EOF");
            } else {
                continue;
            }
        }

        let version = match req.version.unwrap() {
            0 => http::Version::HTTP_10,
            1 => http::Version::HTTP_11,
            _ => panic!("invalid version number"),
        };

        let parsed_length = status.unwrap();
        request = request
            .method(req.method.unwrap())
            .uri(req.path.unwrap())
            .version(version);

        for header in req.headers {
            request = request.header(header.name, std::str::from_utf8(header.value).unwrap());
        }

        stream.consume(parsed_length);
        break;
    }

    let mut length = Length::None;
    if let Some(headers) = request.headers_ref() {
        if let Some(v) = headers.get(http::header::CONTENT_LENGTH) {
            if let Ok(nb) = v.to_str().unwrap().parse::<usize>() {
                length = Length::ContentLength(nb);
            }
        }

        if headers
            .get_all(http::header::TRANSFER_ENCODING)
            .iter()
            .find(|c| util::eq_no_case(c.as_bytes(), "chunked".as_bytes()))
            .is_some()
        {
            length = Length::Chunked(0);
        }
    }

    //println!("finished parsing headers:\n{:?}", request);
    let body = Body {
        stream,
        length,
        at_eof,
    };

    Ok(request.body(body)?)
}

pub fn respond<
    Stream: Read + Write + Debug,
    T: BufRead + Read + HasLength + Debug,
>(
    stream: Stream,
    response: http::Response<T>,
) -> Result<Stream, HttpError> {
    let mut stream = BufWriter::new(stream);
    //println!("sending response:\n{:?}", response);

    // we'are assuming that the reqest line and all headers will fit into the buffer
    write!(&mut stream, "HTTP/1.1 {}\r\n", response.status())?;

    for (name, value) in response.headers() {
        write!(&mut stream, "{}: ", name.as_str())?;
        stream.write_all(value.as_ref())?;
        stream.write_all(&b"\r\n"[..])?;
    }

    if let Some(sz) = response.body().has_length() {
        write!(&mut stream, "Content-Length: {}\r\n", sz)?;
    } else {
        stream.write_all(&b"Transfer-Encoding: Chunked\r\n"[..])?;
    }

    let has_length = (*response.body()).has_length().is_some();
    stream.write_all(&b"\r\n"[..])?;

    let mut body = response.into_body();
    if has_length {
        std::io::copy(&mut body, &mut stream)?;
    } else {
        loop {
            let data = body.fill_buf()?;
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
    //println!("finished sending response");

    let stream = match stream.into_inner() {
        Ok(s) => s,
        Err(_) => panic!(),
    };

    Ok(stream)
}
