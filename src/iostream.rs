use tokio_io::{self,AsyncRead,AsyncWrite};
use tokio_tcp::TcpStream;
use std::net::{SocketAddr,Shutdown};
use futures::{Async,Poll};
use bytes::{BytesMut,BufMut};
use std::time;
use std::io;

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

#[doc(hidden)]
/// Low-level io stream operations
pub trait IoStream: AsyncRead + AsyncWrite + 'static {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()>;

    /// Returns the socket address of the remote peer of this TCP connection.
    fn peer_addr(&self) -> Option<SocketAddr> {
        None
    }

    /// Sets the value of the TCP_NODELAY option on this socket.
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;

    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()>;

    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()>;

    fn read_available(&mut self, buf: &mut BytesMut) -> Poll<(bool, bool), io::Error> {
        let mut read_some = false;
        loop {
            if buf.remaining_mut() < LW_BUFFER_SIZE {
                buf.reserve(HW_BUFFER_SIZE);
            }

            let read = unsafe { self.read(buf.bytes_mut()) };
            match read {
                Ok(n) => {
                    if n == 0 {
                        return Ok(Async::Ready((read_some, true)));
                    } else {
                        read_some = true;
                        unsafe {
                            buf.advance_mut(n);
                        }
                    }
                }
                Err(e) => {
                    return if e.kind() == io::ErrorKind::WouldBlock {
                        if read_some {
                            Ok(Async::Ready((read_some, false)))
                        } else {
                            Ok(Async::NotReady)
                        }
                    } else if e.kind() == io::ErrorKind::ConnectionReset && read_some {
                        Ok(Async::Ready((read_some, true)))
                    } else {
                        Err(e)
                    };
                }
            }
        }
    }

}

impl IoStream for ::tokio_uds::UnixStream {
    #[inline]
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        ::tokio_uds::UnixStream::shutdown(self, how)
    }

    #[inline]
    fn set_nodelay(&mut self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_linger(&mut self, _dur: Option<time::Duration>) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_keepalive(&mut self, _dur: Option<time::Duration>) -> io::Result<()> {
        Ok(())
    }
}

impl IoStream for TcpStream {
    #[inline]
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, how)
    }

    #[inline]
    fn peer_addr(&self) -> Option<SocketAddr> {
        TcpStream::peer_addr(self).ok()
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        TcpStream::set_nodelay(self, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_linger(self, dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_keepalive(self, dur)
    }
}
