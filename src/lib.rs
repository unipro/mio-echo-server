use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::collections::VecDeque;

use mio::net::{TcpListener, TcpStream};
use mio::{Events, Poll, PollOpt, Ready, Token};
use slab::Slab;

pub use failure::Error;

const MAX_CLIENTS: usize = 1024;
const DEFAULT_BUF_SIZE: usize = 1024;

struct Client {
    sock: TcpStream,
    writable: bool,
    bufs: VecDeque<Vec<u8>>,
    pos: usize,
}

impl Client {
    pub fn new(sock: TcpStream) -> Client {
        Client {
            sock,
            writable: false,
            bufs: VecDeque::new(),
            pos: 0,
        }
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.sock.peer_addr().unwrap()
    }

    pub fn register(&mut self, poll: &Poll, index: usize) -> io::Result<()> {
        poll.register(&self.sock, Token(index), Ready::readable(), PollOpt::edge())
    }

    pub fn reregister(&mut self, poll: &Poll, index: usize) -> io::Result<()> {
        if self.bufs.is_empty() == self.writable {
            self.writable = ! self.writable;
            let ready = if self.writable {
                Ready::readable() | Ready::writable()
            } else {
                Ready::readable()
            };
            poll.reregister(&self.sock, Token(index), ready, PollOpt::edge())?;
        }
        Ok(())
    }

    pub fn deregister(&self, poll: &Poll) -> io::Result<()> {
        poll.deregister(&self.sock)
    }

    pub fn read(&mut self) -> io::Result<usize> {
        let mut tot_len = 0;
        let mut rbuf = [0; DEFAULT_BUF_SIZE];

        loop {
            match self.sock.read(&mut rbuf) {
                Ok(0) => return Ok(0),
                Ok(len) => {
                    let mut buf = rbuf.to_vec();
                    if len < DEFAULT_BUF_SIZE {
                        unsafe {
                            buf.set_len(len);
                        }
                    }
                    self.bufs.push_back(buf);
                    tot_len += len;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Socket is not ready anymore, stop reading
                    break;
                }
                e => return e,
            }
        }

        Ok(tot_len)
    }

    pub fn write(&mut self) -> io::Result<usize> {
        let mut tot_len = 0;

        while let Some(buf) = self.bufs.get(0) {
            match self.sock.write(&buf[self.pos..]) {
                Ok(len) => {
                    self.pos += len;
                    if buf.len() == self.pos {
                        self.bufs.pop_front();
                        self.pos = 0;
                    }
                    tot_len += len;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Socket is not ready anymore, stop writing
                    break;
                }
                e => return e,
            }
        }

        Ok(tot_len)
    }
}

pub fn run(addr: &str) -> Result<(), Error> {
    const SERVER_TOKEN: Token = Token(MAX_CLIENTS);

    // Tcp listener
    let server = TcpListener::bind(&addr.parse()?)?;

    let poll = Poll::new()?;

    // Register the listener
    poll.register(&server, SERVER_TOKEN, Ready::readable(), PollOpt::edge())?;

    // Create storage for events
    let mut events = Events::with_capacity(1024);

    // Used to store the clients.
    let mut clients = Slab::with_capacity(MAX_CLIENTS);

    // The main event loop
    loop {
        // Wait for events
        poll.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                SERVER_TOKEN => {
                    // Perform operations in a loop until `WouldBlock` is
                    // encountered.
                    loop {
                        match server.accept() {
                            Ok((sock, addr)) => {
                                if clients.len() < MAX_CLIENTS - 1 {
                                    println!("connection established : {}", addr);
                                    let index = clients.insert(Client::new(sock));
                                    clients.get_mut(index).unwrap().register(&poll, index)?;
                                } else {
                                    eprintln!("too many clients");
                                }
                            }
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                                // Socket is not ready anymore, stop accepting
                                break;
                            }
                            Err(e) => {
                                return Err(e.into());
                            }
                        }
                    }
                }
                Token(index) => {
                    if event.readiness().is_readable() {
                        match clients.get_mut(index).unwrap().read() {
                            Ok(0) => {
                                // Socket is closed, remove it
                                clients.get(index).unwrap().deregister(&poll)?;
                                let client = clients.remove(index);
                                println!("connection closed : {}", client.peer_addr());
                                continue;
                            }
                            Ok(len) => {
                                println!("read {} bytes : {}", len, clients.get(index).unwrap().peer_addr());
                            }
                            Err(e) => {
                                clients.get(index).unwrap().deregister(&poll)?;
                                let client = clients.remove(index);
                                println!("error={} : {}", e, client.peer_addr());
                                continue;
                            }
                        }
                    }

                    match clients.get_mut(index).unwrap().write() {
                        Ok(len) => {
                            clients.get_mut(index).unwrap().reregister(&poll, index)?;
                            println!("write {} bytes : {}", len, clients.get(index).unwrap().peer_addr());
                        }
                        Err(e) => {
                            clients.get(index).unwrap().deregister(&poll)?;
                            let client = clients.remove(index);
                            println!("error={} : {}", e, client.peer_addr());
                            continue;
                        }
                    }
                }
            }
        }
    }
}
