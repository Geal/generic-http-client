use std::io::{self, Read, Write, BufReader, BufRead};
use std::fmt::Debug;

#[derive(Debug)]
pub struct Body<Stream: Read + Debug> {
    pub(crate) stream: BufReader<Stream>,
    pub(crate) length: Length,
    pub(crate) at_eof: bool,
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
                            Err(_invalid_chunk_size) => {
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
