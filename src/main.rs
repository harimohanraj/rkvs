
extern crate mio;
extern crate bytes;

use mio::*;
use mio::tcp::*;
use std::io;
use std::str;
use std::net::{SocketAddr};
use std::collections::{HashMap};
use bytes::{Buf, ByteBuf, MutByteBuf, SliceBuf};

const SERVER_TOKEN: Token = Token(0);

struct Connection {
    socket: TcpStream,
    interest: EventSet,
    mut_buf: Option<MutByteBuf>
}

impl Connection {
    fn new(socket: TcpStream) -> Connection {
        Connection {
            socket: socket,
            interest: EventSet::hup(),
            mut_buf: Some(ByteBuf::mut_with_capacity(2048))
        }
    }

    fn read(&mut self) {
        let mut buf = self.mut_buf.take().unwrap();
        match self.socket.try_read_buf(&mut buf) {
            Ok(None) => {
                println!("CONN: spurious read wakeup");
                self.mut_buf = Some(buf);
            },
            Ok(Some(r)) => {
                println!("CONN: we read {} bytes", r);
                
                // provide some method for processing incoming message
                let bytes = buf.bytes();
                let line = str::from_utf8(bytes).unwrap();
                println!("{}", line);      
            },
            Err(e) => {
                println!("Not implemented; client err={:?}", e); 
            }
        }
    }
}

struct Server {
    socket: TcpListener,
    connections: HashMap<Token, Connection>, 
    token_counter: usize
}
impl Server {
    fn new(addr: &str, event_loop: &mut EventLoop<Server>) -> Server {
        let address = addr.parse::<SocketAddr>().unwrap();
        let server_socket = TcpListener::bind(&address).unwrap();
         
        let new_server = Server {
            socket: server_socket,
            connections: HashMap::new(),
            token_counter: 1
        };

        event_loop.register(&new_server.socket, 
                            SERVER_TOKEN, EventSet::readable(),
                            PollOpt::edge()).unwrap();
        return new_server;
    }

    fn accept(&mut self, event_loop: &mut EventLoop<Server>) {
        let incoming_socket = match self.socket.accept() {
            Err(e) => {
                println!("Accept error: {}", e);
                return;
            },
            Ok(None) => unreachable!(),
            Ok(Some((sock, addr))) => sock
        };
   
        self.token_counter += 1;
        let new_token = Token(self.token_counter);
        let new_conn = Connection::new(incoming_socket);
        self.connections.insert(new_token, new_conn);
        
        event_loop.register(&self.connections[&new_token].socket, 
                            new_token, EventSet::readable(), 
                            PollOpt::edge() | PollOpt::oneshot());           
    }
}

impl Handler for Server {
    type Timeout = usize;
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<Server>, 
             token: Token, events: EventSet) 
    {   
        /*
        match token {
            SERVER_TOKEN => {
                println!("Connected to server at {}", 
                         self.socket.local_addr().unwrap());
                self.accept(event_loop);
            },
            token => {
                println!("Talking to client at {}", 
                         self.connections[&token].socket.peer_addr().unwrap());
                let mut connection = self.connections.get_mut(&token).unwrap();
                connection.read();
                event_loop.reregister(&connection.socket, 
                                      token, EventSet::readable(),
                                      PollOpt::edge() | PollOpt::oneshot()).unwrap();
            } 
        } 
        */
        
        if events.is_readable() {
            match token {
                SERVER_TOKEN => {
                    println!("Connected to server at {}",
                             self.socket.local_addr().unwrap());
                    self.accept(event_loop);
                },
                token => {
                    println!("Talking to connection at {}",
                             self.connections[&token].socket.peer_addr().unwrap());
                    let mut connection = self.connections.get_mut(&token).unwrap();
                    connection.read();
                    event_loop.reregister(&connection.socket,
                                          token, EventSet::readable(),
                                          PollOpt::edge() | PollOpt::oneshot()).unwrap();
                }
            }
        }
        
        /*
        if events.is_writable() {
            match token {
                SERVER_TOKEN => panic!("Received writable for token 0"),
                _ => {
                    println!("Writing to connection");
                    let mut connection = self.connections.get_mut(&token).unwrap();
                    connection.write();
                }
            }
        }
        */
    }   

}

fn main() {
    println!("Starting test server ..");
    let mut event_loop = EventLoop::new().unwrap();
    let mut handler = Server::new("127.0.0.1:8000", &mut event_loop);
    event_loop.run(&mut handler).unwrap();
}
