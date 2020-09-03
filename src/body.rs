use crate::accumulator::AccReader;
use std::fmt::Debug;
use std::io::{self, BufRead, Read, Write};
use log::{info, error};

#[derive(Debug, Clone)]
pub struct Body<Stream: Read + Write + Debug> {
    pub(crate) stream: AccReader<Stream>,
    pub(crate) length: Length,
    pub(crate) at_eof: bool,
}

#[derive(Debug, Clone)]
pub enum Length {
    None,
    //remaining size
    ContentLength(usize),
    // remaining size in the current chunk
    Chunked(usize),
}

impl<Stream: Read + Write + Debug> Body<Stream> {
    pub fn into_inner(self) -> AccReader<Stream> {
        self.stream
    }
}

impl<Stream: Read+Write+Debug> crate::HasLength for Body<Stream> {
    fn has_length(&self) -> Option<usize> {
        match self.length {
            Length::ContentLength(sz) => Some(sz),
            _ => None,
        }
    }
}

impl<Stream: Read + Write + Debug> Read for Body<Stream> {
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

                let mut index = 0usize;
                loop {
                    let bound = std::cmp::min(sz - index, (&buf[index..]).len());
                    if bound == 0 {
                        break;
                    }

                    if bound > self.stream.buffer().len() {
                        //println!("refilling (bound = {}, buffer len: {}, internal buffer len = {})",
                        //bound, buf.len(),
                        //self.stream.buffer().len());
                        if self.at_eof {
                            return Ok(0);
                        }

                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            return Ok(0);
                        } else {
                            //println!("added {} more bytes", data.len());
                        }
                    }

                    // leaving two bytes to check for \r\n
                    let internal_bound = std::cmp::min(bound, self.stream.buffer().len());
                    //println!("remaining chunk size: {}, buffer len: {}, internal buffer len: {}, bound: {}, internal bound: {}", sz - index, buf.len(), self.stream.buffer().len(), bound, internal_bound);

                    let written = (&mut buf[index..index + bound])
                        .write(&self.stream.buffer()[..internal_bound])?;
                    //println!("wrote:{:?}", std::str::from_utf8(&self.stream.buffer()[..written]));
                    self.stream.consume(written);
                    index += written;
                }

                if sz == index {
                    if &self.stream.buffer()[..2] == &b"\r\n"[..] {
                        self.stream.consume(2);
                    } else {
                        return Err(io::Error::new(io::ErrorKind::Other, "invalid chunk end"));
                    }
                }

                //println!(" ==> read {} bytes of chunk", index);

                (Length::Chunked(sz - index), Ok(index))
            }
        };

        self.length = length;

        res
    }
}

impl<Stream: Read+Write+Debug> BufRead for Body<Stream> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let (length, res) = match self.length {
            Length::None => return Ok(&b""[..]),
            Length::ContentLength(sz) => {
                if sz == 0 {
                    return Ok(&b""[..]);
                }

                if self.stream.buffer().len() == 0 {
                    if self.at_eof {
                        return Ok(&b""[..]);
                    } else {
                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            return Ok(&b""[..]);
                        }
                    }
                }

                (Length::ContentLength(sz), &self.stream.buffer()[..sz])
            },
            Length::Chunked(mut sz) => {
                // we need to parse a chunk header
                if sz == 0 {
                    if self.stream.buffer().is_empty() {
                        if self.at_eof {
                            return Ok(&b""[..]);
                        }

                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            return Ok(&b""[..]);
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
                                        return Ok(&b""[..]);
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
                    return Ok(&b""[..]);
                }

                if self.stream.buffer().len() == 0 {
                    if self.at_eof {
                        (Length::Chunked(sz), &b""[..])
                    } else {
                        let data = self.stream.fill_buf()?;

                        if data.is_empty() {
                            self.at_eof = true;
                            (Length::Chunked(sz), &b""[..])
                        } else {
                            let min = std::cmp::min(sz, self.stream.buffer().len());
                            (Length::Chunked(sz), &self.stream.buffer()[..min])
                        }
                    }
                } else {
                    let min = std::cmp::min(sz, self.stream.buffer().len());
                    (Length::Chunked(sz), &self.stream.buffer()[..min])
                }
            }

        };

        self.length = length;

        Ok(res)
    }

    fn consume(&mut self, amt: usize) {
        self.stream.consume(amt);
        self.length = match self.length {
            Length::None => Length::None,
            Length::ContentLength(sz) => {
                if sz >= amt {
                    Length::ContentLength(sz - amt)
                } else {
                    panic!("cannot consume past the content length")
                }
            }
            Length::Chunked(sz) => {
                if sz >= amt {
                    if sz == amt {
                        if self.stream.buffer().is_empty() {
                            if self.at_eof {
                                return;
                            }

                            let data = match self.stream.fill_buf() {
                                Ok(data) => data,
                                Err(e) => {
                                    error!("could not fill buffer: {:?}", e);
                                    return;
                                }
                            };

                            if data.is_empty() {
                                self.at_eof = true;
                                return;
                            }
                        }

                        if self.stream.buffer().len() >= 2 && &self.stream.buffer()[..2] == &b"\r\n"[..] {
                            self.stream.consume(2);
                        } else {
                            error!("could not parse chunk end");
                            //FIXME: we might need a call to fill_buf here
                            panic!("could not parse chunk end");
                        }
                    }
                    Length::Chunked(sz - amt)
                } else {
                    panic!("cannot consume past the chunk length")
                }
            }
        };
    }
}
