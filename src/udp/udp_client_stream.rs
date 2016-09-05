// Copyright 2015-2016 Benjamin Fry <benjaminfry@me.com>
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::fmt;
use std::io;

use futures::{Async, BoxFuture, Future, Map, Poll};
use futures::stream::{Fuse, Stream};
use rand::Rng;
use rand;
use tokio_core;
use tokio_core::{Loop, LoopHandle, Sender, Receiver};
use tokio_core::io::IoFuture;

use ::error::*;
use client::ClientConnection;

pub struct UdpClientStream {
  // TODO: this shouldn't be stored, it's only necessary for the client to setup Ipv4 or Ipv6
  //   binding
  // destination address for all requests
  name_server: SocketAddr,
  //
  socket: tokio_core::UdpSocket,
  outbound_messages: Fuse<Receiver<Vec<u8>>>,
  message_sender: Sender<Vec<u8>>,
  outbound_opt: Option<Vec<u8>>,
}

lazy_static!{
  static ref IPV4_ZERO: Ipv4Addr = Ipv4Addr::new(0,0,0,0);
  static ref IPV6_ZERO: Ipv6Addr = Ipv6Addr::new(0,0,0,0,0,0,0,0);
}

impl UdpClientStream {
  /// it is expected that the resolver wrapper will be responsible for creating and managing
  ///  new UdpClients such that each new client would have a random port (reduce chance of cache
  ///  poisoning)
  pub fn new(name_server: SocketAddr, loop_handle: LoopHandle) -> BoxFuture<Self, io::Error> {
    let (message_sender, outbound_messages) = loop_handle.clone().channel();

    // TODO: allow the bind address to be specified...
    // constructs a future for getting the next randomly bound port to a UdpSocket
    let next_socket = Self::next_bound_local_address(&name_server, loop_handle);

    // This set of futures collapses the next udp socket into a stream which can be used for
    //  sending and receiving udp packets.
    let stream = next_socket.map(move |socket| {
      socket.join(outbound_messages).map(move |(socket, rx)| {
        UdpClientStream {
          name_server: name_server,
          socket: socket,
          outbound_messages: rx.fuse(),
          message_sender: message_sender,
          outbound_opt: None,
        }
      })
    }).flatten();

    stream.boxed()
  }

  /// Creates a future for randomly binding to a local socket address for client connections.
  fn next_bound_local_address(name_server: &SocketAddr, loop_handle: LoopHandle) -> NextRandomUdpSocket {
    let zero_addr: IpAddr = match *name_server {
      SocketAddr::V4(..) => IpAddr::V4(*IPV4_ZERO),
      SocketAddr::V6(..) => IpAddr::V6(*IPV6_ZERO),
    };

    NextRandomUdpSocket{ bind_address: zero_addr, loop_handle: loop_handle }
  }

  pub fn send(&self, buffer: Vec<u8>) -> io::Result<()> {
    self.message_sender.send(buffer)
  }
}

impl Stream for UdpClientStream {
  type Item = Vec<u8>;
  type Error = io::Error;

  fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
    // this will not accept incoming data while there is data to send
    //  makes this self throttling.
    loop {
      if let Some(ref buffer) = self.outbound_opt {
        // will return if the socket will block
        try_nb!(self.socket.send_to(buffer, &self.name_server));
      }

      // if we got here, then any message was sent.
      self.outbound_opt = None;

      match try!(self.outbound_messages.poll()) {
        Async::Ready(Some(buffer)) => {
          match try!(self.socket.poll_write()) {
            Async::NotReady => return Ok(Async::NotReady),
            Async::Ready(_) => {
              self.outbound_opt = Some(buffer);
            },
          }
        },
        Async::NotReady | Async::Ready(None) => break,
      }
    }

    // check the reciever, if it's closed, we are done, b/c there will be no more requests
    if self.outbound_messages.is_done() {
      return Ok(Async::Ready(None));
    }

    // For QoS, this will only accept one message and output that
    // recieve all inbound messages

    // TODO: this should match edns settings
    let mut buf = [0u8; 2048];

    // TODO: should we drop this packet if it's not from the same src as dest?
    let (len, src) = try_nb!(self.socket.recv_from(&mut buf));
    if src != self.name_server {
      debug!("{} does not match name_server: {}", src, self.name_server)
    }

    Ok(Async::Ready(Some(buf.iter().take(len).cloned().collect())))
  }
}

struct NextRandomUdpSocket {
  bind_address: IpAddr,
  loop_handle: LoopHandle,
}

impl Future for NextRandomUdpSocket {
  type Item = IoFuture<tokio_core::UdpSocket>;
  type Error = io::Error;

  /// polls until there is an available next random UDP port.
  ///
  /// if there is no port available after 10 attempts, returns NotReady
  fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
    let mut rand = rand::thread_rng();

    for attempt in 0..10 {
      let zero_addr = SocketAddr::new(self.bind_address, rand.gen_range(1025_u16, u16::max_value()));

      match UdpSocket::bind(&zero_addr) {
        Ok(socket) => return Ok(Async::Ready(tokio_core::UdpSocket::from_socket(socket, self.loop_handle.clone()))),
        Err(err) => debug!("unable to bind port, attempt: {}: {}", attempt, err),
      }
    }

    warn!("could not get next random port, delaying");

    // returning NotReady here, perhaps the next poll there will be some more socket available.
    Ok(Async::NotReady)
  }
}