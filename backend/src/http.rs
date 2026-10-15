//! 局域网HTTP连接与有界请求线程池；鉴权和写入由统一服务层处理。
use crate::{
    PanelError, Result,
    boundary::{self, Lan},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub trait Handler: Send + Sync {
    fn handle(
        &self,
        path: &str,
        body: Value,
        headers: &[(String, String)],
        peer: Ipv4Addr,
    ) -> Result<Value>;
}
struct Request {
    path: String,
    body: Value,
    headers: Vec<(String, String)>,
    keep: bool,
}
fn invalid() -> PanelError {
    PanelError::new("INVALID_REQUEST", 400)
}
fn timeout(stream: &TcpStream, deadline: Instant) -> Result<()> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or_else(invalid)?;
    stream.set_read_timeout(Some(left)).map_err(|_| invalid())
}
fn line(stream: &mut BufReader<TcpStream>, budget: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut out = vec![];
    while out.len() < budget {
        timeout(stream.get_ref(), deadline)?;
        let raw = stream.fill_buf().map_err(|_| invalid())?;
        if raw.is_empty() {
            return Err(invalid());
        }
        let n = raw
            .iter()
            .position(|b| *b == b'\n')
            .map_or(raw.len(), |i| i + 1)
            .min(budget - out.len());
        out.extend_from_slice(&raw[..n]);
        stream.consume(n);
        if out.ends_with(b"\r\n") {
            return Ok(out);
        }
        if out.ends_with(b"\n") {
            return Err(invalid());
        }
    }
    Err(PanelError::new("HEADERS_TOO_LARGE", 431))
}
fn read_request(stream: &mut BufReader<TcpStream>, lan: Lan, peer: Ipv4Addr) -> Result<Request> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let first = line(stream, 1024, deadline)?;
    let first = std::str::from_utf8(&first).map_err(|_| invalid())?;
    let words = first
        .trim_end_matches("\r\n")
        .split(' ')
        .collect::<Vec<_>>();
    if words.len() != 3 || words[2] != "HTTP/1.1" {
        return Err(invalid());
    }
    let method = words[0];
    let path = words[1].to_owned();
    if !(method == "GET" && path == "/health"
        || method == "POST" && boundary::POST_ROUTES.contains(&path.as_str()))
    {
        return Err(PanelError::new("NOT_FOUND", 404));
    }
    let mut headers = vec![];
    let mut size = first.len();
    loop {
        let raw = line(stream, 8192usize.saturating_sub(size), deadline)?;
        size += raw.len();
        if raw == b"\r\n" {
            break;
        }
        if headers.len() >= 64 {
            return Err(PanelError::new("HEADERS_TOO_LARGE", 431));
        }
        let raw = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        let (key, value) = raw
            .trim_end_matches("\r\n")
            .split_once(':')
            .ok_or_else(invalid)?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b < 32 && b != 9 || b == 127)
        {
            return Err(invalid());
        }
        headers.push((key.to_owned(), value.trim_matches([' ', '\t']).to_owned()));
    }
    lan.check(peer, &headers)?;
    if headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("expect"))
    {
        return Err(PanelError::new("EXPECTATION_DENIED", 417));
    }
    let body = if method == "POST" {
        let size = boundary::content_length(&headers)?;
        let mut raw = vec![0; size];
        let mut offset = 0;
        while offset < size {
            timeout(stream.get_ref(), deadline)?;
            let n = stream.read(&mut raw[offset..]).map_err(|_| invalid())?;
            if n == 0 {
                return Err(invalid());
            }
            offset += n;
        }
        boundary::decode_json(&raw)?
    } else {
        if headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding")
                || k.eq_ignore_ascii_case("content-length") && v != "0"
        }) {
            return Err(invalid());
        }
        json!({})
    };
    let keep = !headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("connection")
            && v.split(',').any(|s| s.trim().eq_ignore_ascii_case("close"))
    });
    Ok(Request {
        path,
        body,
        headers,
        keep,
    })
}
fn response(stream: &mut TcpStream, result: Result<Value>, keep: bool) -> Result<()> {
    let (code, value) = match result {
        Ok(v) => (200, json!({"ok":true,"data":v})),
        Err(e) => (e.status, json!({"ok":false,"error":e.details()})),
    };
    let body = serde_json::to_vec(&value).map_err(|_| invalid())?;
    if body.len() > 262144 {
        return Err(PanelError::new("SNAPSHOT_TOO_LARGE", 503));
    }
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| invalid())?;
    let head = format!(
        "HTTP/1.1 {code} Result\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len(),
        if keep { "keep-alive" } else { "close" }
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|_| invalid())
}
pub fn connection(
    stream: TcpStream,
    lan: Lan,
    peer: Ipv4Addr,
    handler: &dyn Handler,
) -> Result<()> {
    connection_until(stream, lan, peer, handler, &AtomicBool::new(false))
}
fn connection_until(
    stream: TcpStream,
    lan: Lan,
    peer: Ipv4Addr,
    handler: &dyn Handler,
    stop: &AtomicBool,
) -> Result<()> {
    stream.set_nodelay(true).map_err(|_| invalid())?;
    let mut stream = BufReader::new(stream);
    for count in 0..32 {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match read_request(&mut stream, lan, peer) {
            Ok(request) => {
                let keep = request.keep && count < 31;
                let result = handler.handle(&request.path, request.body, &request.headers, peer);
                response(stream.get_mut(), result, keep)?;
                if !keep {
                    return Ok(());
                }
            }
            Err(e) => {
                let _ = response(stream.get_mut(), Err(e), false);
                return Ok(());
            }
        }
    }
    Ok(())
}
pub fn serve(
    listener: TcpListener,
    lan: Lan,
    handler: Arc<dyn Handler>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    listener.set_nonblocking(true).map_err(|_| invalid())?;
    let (send, receive) = mpsc::sync_channel::<TcpStream>(8);
    let receive = Arc::new(Mutex::new(receive));
    let mut workers = vec![];
    for _ in 0..8 {
        let queue = receive.clone();
        let handler = handler.clone();
        let stopping = stop.clone();
        workers.push(std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let next = queue
                    .lock()
                    .ok()
                    .and_then(|queue| queue.recv_timeout(Duration::from_millis(100)).ok());
                if let Some(stream) = next {
                    if let Ok(std::net::SocketAddr::V4(peer)) = stream.peer_addr() {
                        let _ =
                            connection_until(stream, lan, *peer.ip(), handler.as_ref(), &stopping);
                    }
                }
            }
        }));
    }
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(mpsc::TrySendError::Full(stream)) = send.try_send(stream) {
                    drop(stream);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }
    }
    drop(send);
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}
