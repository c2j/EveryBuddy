//! Reproduces the accept loop in `crates/eb/src/preview_server.rs`.
//!
//! One thread, one `read` of at most 2048 bytes, then the response. A client
//! that connects and does not finish the request occupies the loop. Bind
//! failure is the same `else { return }` the production function uses.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut request = [0u8; 2048];
        let Ok(read) = stream.read(&mut request) else { continue };
        let request = String::from_utf8_lossy(&request[..read]);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let body = format!("path={path} bytes={read}");
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
    }
}

fn listen() -> (TcpListener, std::net::SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    (listener, addr)
}

fn connect_retry(addr: std::net::SocketAddr) -> TcpStream {
    for _ in 0..50 {
        if let Ok(stream) = TcpStream::connect(addr) {
            return stream;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("connect {addr}");
}

fn main() {
    let (listener, addr) = listen();
    thread::spawn(move || serve(listener));

    let mut hold = connect_retry(addr);
    let mut second = connect_retry(addr);
    second
        .set_read_timeout(Some(Duration::from_millis(400)))
        .expect("read timeout");
    let wrote = second
        .write_all(b"GET /second HTTP/1.1\r\nHost: probe\r\n\r\n")
        .is_ok();
    let mut buf = [0u8; 64];
    match second.read(&mut buf) {
        Err(error) => println!(
            "HAZARD preview-starve CONFIRMED held connection blocks the next response (write_queued={wrote}, read={error})"
        ),
        Ok(0) => println!("HAZARD preview-starve CONFIRMED second request closed with no response"),
        Ok(n) => println!(
            "HAZARD preview-starve REFUTED second request returned {n} bytes: {}",
            String::from_utf8_lossy(&buf[..n])
        ),
    }
    let _ = hold.write_all(b"GET /hold HTTP/1.1\r\nHost: probe\r\n\r\n");
    drop(hold);
    drop(second);

    let (listener, addr) = listen();
    match TcpListener::bind(addr) {
        Err(error) => println!(
            "HAZARD preview-silent-bind CONFIRMED second bind of {addr} failed ({error}); production drops this with else {{ return }}"
        ),
        Ok(_) => println!(
            "HAZARD preview-silent-bind REFUTED second bind of {addr} succeeded on this OS"
        ),
    }
    drop(listener);

    let (listener, addr) = listen();
    thread::spawn(move || serve(listener));
    let mut client = connect_retry(addr);
    client
        .write_all(b"GET /file?id=0 HTTP/1.1\r\nHost: probe\r\n\r\n")
        .expect("write");
    let mut response = Vec::new();
    client.read_to_end(&mut response).expect("read body");
    let text = String::from_utf8_lossy(&response);
    if text.contains("path=/file?id=0") && !text.to_ascii_lowercase().contains("authorization") {
        println!("HAZARD preview-no-auth CONFIRMED local client read the body with no token");
    } else {
        println!("HAZARD preview-no-auth REFUTED {text}");
    }

    let (listener, addr) = listen();
    thread::spawn(move || serve(listener));
    let mut client = connect_retry(addr);
    client
        .write_all(b"POST /event HTTP/1.1\r\nHost: probe\r\nContent-Length: 11\r\n\r\n")
        .expect("write headers");
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let mut early = [0u8; 512];
    let n = client.read(&mut early).unwrap_or(0);
    let early_text = String::from_utf8_lossy(&early[..n]);
    let _ = client.write_all(b"hello-event");
    if early_text.contains("bytes=") && !early_text.contains("hello-event") {
        println!(
            "HAZARD preview-single-read CONFIRMED the loop answered from the first read, before the body: {}",
            early_text.replace('\r', " ").replace('\n', " | ")
        );
    } else {
        println!(
            "HAZARD preview-single-read UNEXPECTED {}",
            early_text.replace('\r', " ").replace('\n', " | ")
        );
    }
}
