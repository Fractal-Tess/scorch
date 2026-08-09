use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tracing::debug;
use url::Url;

use crate::{error::EngineError, security::SecurityPolicy};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

pub struct SafeProxy {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl SafeProxy {
    pub async fn start(security: SecurityPolicy) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let security = Arc::new(security);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    break;
                };
                let security = Arc::clone(&security);
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, security).await {
                        debug!(%peer, %error, "browser proxy rejected connection");
                    }
                });
            }
        });
        Ok(Self { address, task })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for SafeProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_connection(
    mut client: TcpStream,
    security: Arc<SecurityPolicy>,
) -> Result<(), EngineError> {
    let request = read_headers(&mut client).await?;
    let header_end = find_header_end(&request)
        .ok_or_else(|| EngineError::UnsafeUrl("invalid proxy request".into()))?;
    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|_| EngineError::UnsafeUrl("proxy request was not valid text".into()))?;
    let first_line = headers
        .lines()
        .next()
        .ok_or_else(|| EngineError::UnsafeUrl("empty proxy request".into()))?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method.eq_ignore_ascii_case("CONNECT") {
        tunnel_connect(client, target, &request[header_end..], &security).await
    } else {
        forward_http(
            client,
            method,
            target,
            headers,
            &request[header_end..],
            &security,
        )
        .await
    }
}

async fn tunnel_connect(
    mut client: TcpStream,
    authority: &str,
    buffered: &[u8],
    security: &SecurityPolicy,
) -> Result<(), EngineError> {
    let target = security.validate(&format!("https://{authority}/")).await?;
    let mut upstream = connect_pinned(&target.addresses).await?;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|error| EngineError::Fetch(error.to_string()))?;
    if !buffered.is_empty() {
        upstream
            .write_all(buffered)
            .await
            .map_err(|error| EngineError::Fetch(error.to_string()))?;
    }
    timeout(
        IO_TIMEOUT,
        tokio::io::copy_bidirectional(&mut client, &mut upstream),
    )
    .await
    .map_err(|_| EngineError::Timeout)?
    .map_err(|error| EngineError::Fetch(error.to_string()))?;
    Ok(())
}

async fn forward_http(
    mut client: TcpStream,
    method: &str,
    target: &str,
    raw_headers: &str,
    buffered: &[u8],
    security: &SecurityPolicy,
) -> Result<(), EngineError> {
    let validated = security.validate(target).await?;
    let mut upstream = connect_pinned(&validated.addresses).await?;
    let origin_form = origin_form(&validated.url);
    let mut outgoing = format!("{method} {origin_form} HTTP/1.1\r\n");
    for line in raw_headers.lines().skip(1) {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:") || lower.starts_with("connection:") {
            continue;
        }
        if !line.is_empty() {
            outgoing.push_str(line);
            outgoing.push_str("\r\n");
        }
    }
    outgoing.push_str("Connection: close\r\n\r\n");
    upstream
        .write_all(outgoing.as_bytes())
        .await
        .map_err(|error| EngineError::Fetch(error.to_string()))?;
    upstream
        .write_all(buffered)
        .await
        .map_err(|error| EngineError::Fetch(error.to_string()))?;

    timeout(
        IO_TIMEOUT,
        tokio::io::copy_bidirectional(&mut client, &mut upstream),
    )
    .await
    .map_err(|_| EngineError::Timeout)?
    .map_err(|error| EngineError::Fetch(error.to_string()))?;
    Ok(())
}

async fn connect_pinned(addresses: &[SocketAddr]) -> Result<TcpStream, EngineError> {
    let mut last_error = None;
    for address in addresses {
        match timeout(IO_TIMEOUT, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some("connection timed out".into()),
        }
    }
    Err(EngineError::Fetch(
        last_error.unwrap_or_else(|| "target has no usable address".into()),
    ))
}

async fn read_headers(stream: &mut TcpStream) -> Result<Vec<u8>, EngineError> {
    let mut request = Vec::with_capacity(4096);
    loop {
        if request.len() >= MAX_HEADER_BYTES {
            return Err(EngineError::UnsafeUrl(
                "proxy headers were too large".into(),
            ));
        }
        let mut buffer = [0_u8; 4096];
        let count = timeout(IO_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|error| EngineError::Fetch(error.to_string()))?;
        if count == 0 {
            return Err(EngineError::Fetch("proxy client disconnected".into()));
        }
        request.extend_from_slice(&buffer[..count]);
        if find_header_end(&request).is_some() {
            return Ok(request);
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn origin_form(url: &Url) -> String {
    let mut value = url.path().to_owned();
    if value.is_empty() {
        value.push('/');
    }
    if let Some(query) = url.query() {
        value.push('?');
        value.push_str(query);
    }
    value
}
