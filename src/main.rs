#[macro_use] extern crate log;
extern crate env_logger;
extern crate mio;
extern crate bytes;
extern crate slab;

use mio::*;
use mio::tcp::*;
use slab::*;

use std::io;
use std::io::Result;
use std::io::{Read, Write, Error, ErrorKind};
use std::str;
use std::collections::{HashMap};
use std::net::{SocketAddr};

fn main() {
	// initialize logger
	env_logger::init().ok().expect("Failed to init logger");

	// launch server instance
	let mut rkvs = RKVS::new("127.0.0.1:8000");
	rkvs.run(); 
}

#[derive(Debug)]
enum RkvsError {
    Io(io::Error),
}

// Connection is the type that handles interfacing with incoming connections. All
// logic for dealing with reading from sockets, writing to sockets, and managing 
// socket state via the event loop is handled by this struct and its methods.
struct Connection {
	socket: TcpStream,
    token: Option<Token>,
	interest: EventSet,	
    buffer: Vec<u8>,
    write_buf: Vec<u8>
}

impl Connection {
	fn new(socket: TcpStream) -> Connection {         
        Connection {
            socket: socket,
            token: None,
            interest: EventSet::hup(),     
            buffer: Vec::with_capacity(128),
            write_buf: Vec::with_capacity(128)
        }
    }
    
    fn register(&mut self, event_loop: &mut EventLoop<Server>) -> Result<()> {
        self.interest.insert(EventSet::readable());
        event_loop.register(&self.socket, self.token.unwrap(), self.interest, 
                            PollOpt::edge() | PollOpt::oneshot())
                  .or_else(|e| {
                      error!("Failed to register {:?}, {:?}", self.token, e);
                      Err(e)
                  })
    }

    fn reregister(&mut self, event_loop: &mut EventLoop<Server>) -> Result<()> {
        event_loop.reregister(&self.socket, self.token.unwrap(),
                              self.interest, PollOpt::edge() | PollOpt::oneshot())
                  .or_else(|e| {
                      error!("Failed to reregister {:?}, {:?}", self.token, e);
                      Err(e)
                  })
    }

	fn readable(&mut self) -> io::Result<Vec<Command>> {  
        let mut commands = Vec::new();
        let mut buffer = &mut self.buffer;
        let read_from = buffer.len();
        
        match self.socket.read_to_end(&mut buffer) {
            Ok(0) => {
                debug!("CONN => spurious read, no bytes available");
                return Ok(commands);
            },
            Ok(_) => {
                debug!("CONN => matched on some bytes");
                let thing = str::from_utf8(&buffer);
                println!("{:?}", thing);
            },
            Err(ref error) if error.kind() == ErrorKind::WouldBlock => (),
            Err(e) => {
                error!("CONN => Failed to read for {:?}, error: {}", self.token, e);
                return Err(e);
            } 
        }
    
        let mut lo = 0;
        for (hi, &c) in buffer[read_from..].iter().enumerate() {
            if c == '\n' as u8 {
                let line = &buffer[lo..hi];
                commands.push(Command::from_bytes(line));
                lo = hi + 1;
            }
        }
        // remove readable event
        self.interest.remove(EventSet::readable());
        buffer.drain(..lo).count();
        Ok(commands)
    }

    fn writable(&mut self) -> io::Result<()> {
        debug!("CONN => Writing to client");
        let mut idx = 0;
        while idx < self.write_buf.len() {
            match self.socket.write(&self.write_buf[idx..]) {
                Ok(0) => return Err(Error::new(ErrorKind::WriteZero, "Unable to write to socket")),
                Ok(n) => idx += n,
                Err(ref error) if error.kind() == ErrorKind::WouldBlock => {
                    self.write_buf.drain(0..idx).count();
                    return Ok(());
                },
                Err(error) => return Err(error)
            }
        }
        self.interest.remove(EventSet::writable());
        self.write_buf.clear();
        Ok(())
    }

	fn send(&mut self, message: String) {
        self.interest.insert(EventSet::writable());
        self.write_buf.extend(message.as_bytes().iter());
        self.write_buf.push('\n' as u8);
    }
}


// Server manages incoming connections by associating them with a unique Token. It 
// also contains functionality for managing its registration in the event loop. 
struct Server {
	socket: TcpListener,
	connections: Slab<Connection, Token>,
	token: Token,
    db: HashMap<String, String>
}

impl Server {
	fn new(address: SocketAddr) -> Server { 
        let server_socket = TcpListener::bind(&address).ok().unwrap();
        
        Server {
            socket: server_socket,
            connections: Slab::new_starting_at(Token(2), 128),
            token: Token(1),
            db: HashMap::new()
        }    
    }
 
	fn accept(&mut self, event_loop: &mut EventLoop<Server>) {
        debug!("Server => Accepting new client connection.");
        /*
        let incoming_conn = try!(self.socket.accept());
        let incoming_sock = incoming_conn.ok_or("This is an error!")
                                         .and_then(|(sock,addr)| { sock })
                                         .unwrap_or_else(|e| { error!("{:?}",e); });
        */
        
        //let incoming_conn = self.socket.accept().unwrap().unwrap();
        let incoming_conn = match self.socket.accept() {
            Ok(None) => return,
            Ok(Some(stream)) => stream,
            Err(e) => return,
        };
        let incoming_sock = incoming_conn.0;
        let new_conn = Connection::new(incoming_sock);

        /*
        debug!("SERVER => Registering client with event loop.");
        let new_conn = Connection::new(incoming_sock);*/
        let token = self.connections.insert(new_conn)
                        .ok().expect("SERVER => Failed to add connection to slab.");
        self.connections[token].token = Some(token);
        self.connections.get_mut(token)
                        .unwrap()
                        .register(event_loop)
                        .ok()
                        .expect("SERVER => Failed to register client.");
        
        debug!("SERVER => Re-registering server with event loop.");
        self.reregister(event_loop).ok().expect("SERVER => Failed to reregister server");
        ()
    }

    fn conn_writable(&mut self, token: Token) -> io::Result<()> {
        debug!("SERVER => Forward writable event for token {:?}", token);
        self.connections.get_mut(token).unwrap()
            .writable().unwrap();

        Ok(())
    }

    fn conn_readable(&mut self, token: Token) -> io::Result<()> {
        debug!("SERVER => Forward readable event for token {:?}", token);
        let commands = self.connections
                          .get_mut(token)
                          .unwrap()
                          .readable()
                          .unwrap();

        for command in commands {
            info!("SERVER => Received command from {:?}", token);
            match command {
                Command::GET(key) => {  
                    let value = self.db.get(&key).unwrap();     
                    let message = format!("{}", value);
                    self.connections.get_mut(token)
                        .unwrap()
                        .send(message);
                    // would be nice to notify server about what happened
                },
                Command::PUT(key, value) => {
                    // info!("Put value {} in key {} -- overwrite", &value, &key);
                    self.db.insert(key,value);
                    let message = format!("Key inserted");
                    self.connections.get_mut(token)
                        .unwrap()
                        .send(message.to_string());
                    // ditto
                },
                Command::ERR => {
                    info!("SERVER => Command error!");
                    let message = "Error, this is not a valid command.";
                    self.connections.get_mut(token)
                        .unwrap()
                        .send(message.to_string());
                    // ditto
                }
            }
        }
        Ok(())
    }

    fn register(&mut self, event_loop: &mut EventLoop<Server>) -> io::Result<()> {
        event_loop.register(&self.socket,
                            self.token,
                            EventSet::readable(),
                            PollOpt::edge() | PollOpt::oneshot())
                  .or_else(|e| {
                      error!("SERVER => Failed to register server {:?}, {:?}", self.token, e);
                      Err(e)
                  })       
    }

    fn reregister(&mut self, event_loop: &mut EventLoop<Server>) -> io::Result<()> { 
        event_loop.reregister(&self.socket,
                              self.token,
                              EventSet::readable(),
                              PollOpt::edge() | PollOpt::oneshot())
                  .or_else(|e| {
                      error!("SERVER => Failed to reregister server {:?}, {:?}", self.token, e);
                      Err(e)
                  })
    }    
}

impl Handler for Server {
	type Timeout = usize;
	type Message = ();
	
    fn ready(&mut self, event_loop: &mut EventLoop<Server>, token: Token, events: EventSet) {
        debug!("MIO => EventLoop ready for token, events: {:?}, {:?}", token, events);
        if events.is_readable() {    
            if self.token == token {           
                self.accept(event_loop);
            }
            else {
                self.conn_readable(token)
                    .and_then(|_| self.connections
                              .get_mut(token)
                              .unwrap()
                              .reregister(event_loop))
                    .unwrap_or_else(|e| {
                        warn!("MIO => Read event failed for {:?}, {:?}", token, e);
                    });
            }
        }

        if events.is_writable() {
            if self.token == token {
                error!("Shouldn't be getting a writable event for server token");
            }
            else {
                self.conn_writable(token)
                    .and_then(|_| self.connections
                              .get_mut(token)
                              .unwrap()
                              .reregister(event_loop))
                    .unwrap_or_else(|e| {
                        warn!("MIO => Write event failed for {:?}, {:?}", token, e);
                    });
            }
        }
    }
}


#[derive(Debug)]
enum Command {
    GET(String),
    PUT(String, String),
    ERR,
}

impl Command {
    fn from_bytes(bytes: &[u8]) -> Command {
        let line = match str::from_utf8(bytes) {
            Ok(chars) => chars,
            Err(..) => { info!("Error: decode"); return Command::ERR }
        };
        
        let words: Vec<&str> = line.split_whitespace().collect();
        let len = words.len();
        if len < 2 { info!("Incomplete command"); return Command::ERR }
        
        match words[0] {
            "GET" => {
                if len != 2 { info!("Misspecified GET"); Command::ERR }
                else { Command::GET(words[1].to_owned()) }
            },
            "PUT" => {
                if len != 3 { info!("Misspecified PUT, '{}'", str::from_utf8(bytes).unwrap());
                    Command::ERR }
                else { Command::PUT(words[1].to_owned(), words[2].to_owned()) }
            },
            _ => { info!("Unknown command: {}", words[0]); Command::ERR }
        }
    }
}





// RKVS is a struct that handles building the event loop and passing it to the server
// and connection instances. This hides away all those details nicely, similar to the
// echo server implementation in the mio repo.
struct RKVS {
	server: Server
}

impl RKVS {
	fn new(addr: &str) -> RKVS { 
        let address = addr.parse::<SocketAddr>()
                          .ok().expect("RKVS => Failed to parse host:port string.");
        let server = Server::new(address);    
        RKVS { server: server }
    }

	fn run(&mut self) {
        let mut event_loop = EventLoop::new().unwrap();
        self.server.register(&mut event_loop).ok().expect("Something fucked, server didn't reg");
        event_loop.run(&mut self.server).ok().expect("RKVS => Event loop failed to start.");
    }
}

