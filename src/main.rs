

use std::net::{TcpListener, TcpStream};
use std::net::SocketAddr;
use std::thread;


struct KeyValue;

fn main() {

    let address = "127.0.0.1:8000".parse::<SocketAddr>().unwrap();
    let listener = TcpListener::bind(&address).unwrap();
    
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move|| {
                    println!("Connected!")
                });
            },
            Err(e) => { println!("Connection failed.") }
        }
    }
    drop(listener);
}
