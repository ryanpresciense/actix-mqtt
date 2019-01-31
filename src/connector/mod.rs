use crate::iostream::IoStream;
use actix::actors::resolver::{Resolve, Resolver, ResolverError};
use actix::fut::{WrapFuture};
use actix::{Actor, ActorResponse, Context, Handler, MailboxError, Message};
use failure::Fail;
use futures::{Future,  Poll};
use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, Shutdown, SocketAddr};
use std::time;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

#[derive(Debug)]
pub enum Connect {
    Tcp(Resolve),
}

#[derive(Debug, Fail)]
pub enum ConnectError {
    #[fail(display = "Resolver")]
    Resolver(#[cause] ResolverError),
    #[fail(display = "Mailbox")]
    Mailbox(#[cause] MailboxError),
    #[fail(display = "Io")]
    Io(#[cause] io::Error),
    #[fail(display = "NsLookup")]
    NsLookup,
}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self {
        ConnectError::Io(e)
    }
}

impl Message for Connect {
    type Result = Result<Connection, ConnectError>;
}

#[derive(Debug)]
pub struct Connector {}

impl Connector {
    fn resolve(addr: Resolve) -> impl Future<Item = VecDeque<SocketAddr>, Error = ConnectError> {
        let resolver = actix::System::current().registry().get::<Resolver>();
        resolver
            .send(addr)
            .map_err(ConnectError::Mailbox)
            .and_then(|x| x.map_err(ConnectError::Resolver))
    }

    fn connect(
        addrs: VecDeque<SocketAddr>,
    ) -> impl Future<Item = Connection, Error = ConnectError> {
        TcpStream::connect(&addrs[0])
            .map_err(ConnectError::Io)
            .and_then(|connection| {
                connection
                    .peer_addr()
                    .map(|peer| Connection::new(peer.into(), Box::new(connection)))
                    .map_err(ConnectError::Io)
            })
    }
}

impl Actor for Connector {
    type Context = Context<Self>;
}

impl Handler<Connect> for Connector {
    type Result = ActorResponse<Connector, Connection, ConnectError>;

    fn handle(&mut self, connect: Connect, _ctx: &mut Self::Context) -> Self::Result {
        match connect {
            Connect::Tcp(resolve) => ActorResponse::r#async(
                Self::resolve(resolve)
                    .and_then(Self::connect)
                    .into_actor(self),
            ),
        }
    }
}

#[derive(Debug)]
pub enum ConnectionName {
    HostAndPort(IpAddr, u16),
    Other(String),
}

impl From<SocketAddr> for ConnectionName {
    fn from(addr: SocketAddr) -> Self {
        ConnectionName::HostAndPort(addr.ip(), addr.port())
    }
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
    fn new(name: ConnectionName, stream: Box<IoStream + Send>) -> Self {
        Connection { name, stream }
    }

    pub fn stream(&mut self) -> &mut IoStream {
        &mut *self.stream
    }

    pub fn from_stream<T: IoStream + Send, N: AsRef<str>>(name: N, io: T) -> Connection {
        Connection::new(
            ConnectionName::Other(name.as_ref().to_owned()),
            Box::new(io),
        )
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
