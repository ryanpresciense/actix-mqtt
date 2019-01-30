use actix::{Actor,Message,Context};
use actix::actors::resolver::{Resolver,Resolve};
use futures::{Poll};
use tokio_io::{AsyncRead,AsyncWrite};
use std::net::{IpAddr,Shutdown};
use crate::iostream::IoStream;
use std::io;
use std::time;

#[derive(Debug)]
pub struct Connect {
    to: Resolve,
}

#[derive(Debug)]
pub struct Connector {
}

impl Actor for Connector {
    type Context = Context<Self>;
}


#[derive(Debug)]
pub enum ConnectionName {
    HostAndPort(IpAddr,u16),
    Other(String)
}


pub struct Connection {
    name: ConnectionName,
    stream: Box<IoStream + Send>,
}

impl ::std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        write!(f, "Connection {:?}", self.name)
    }
}

impl Connection {
    fn new(name: ConnectionName,stream: Box<IoStream + Send>) -> Self {
        Connection {
            name,
            stream
        }
    }

    pub fn stream(&mut self) -> &mut IoStream {
        &mut *self.stream
    }

    pub fn from_stream<T: IoStream + Send,N: AsRef<str>>(name: N,io: T) -> Connection {
        Connection::new(ConnectionName::Other(name.as_ref().to_owned()), Box::new(io))
    }
}

impl IoStream for Connection {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        IoStream::shutdown(&mut *self.stream, how)
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        IoStream::set_nodelay(&mut *self.stream, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        IoStream::set_linger(&mut *self.stream, dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        IoStream::set_keepalive(&mut *self.stream, dur)
    }
}

impl io::Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl AsyncRead for Connection {}

impl io::Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl AsyncWrite for Connection {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.stream.shutdown()
    }
}
