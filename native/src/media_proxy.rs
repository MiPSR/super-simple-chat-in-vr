//! Bounded, client-local HTTP/CONNECT relay. DNS is resolved once, checked and
//! pinned to a public IP before connecting. TLS remains end-to-end with mpv.
use crate::media_security::public_ip;
use hickory_resolver::TokioResolver;
use std::{
    io,
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Semaphore, oneshot},
    task::JoinSet,
    time::timeout,
};
use url::{Host, Url};

pub struct MediaProxy {
    pub address: SocketAddr,
    pub fixture: Option<SocketAddr>,
    error: Arc<Mutex<String>>,
    stop: Option<oneshot::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for MediaProxy {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn fixture_origin() -> io::Result<Option<SocketAddr>> {
    let Ok(value) = std::env::var("SSC_TEST_MEDIA_ORIGIN") else {
        return Ok(None);
    };
    let url = Url::parse(&value).map_err(|_| invalid("Invalid test media origin"))?;
    let ip = match url.host() {
        Some(Host::Ipv4(ip)) if ip.is_loopback() => ip.into(),
        Some(Host::Ipv6(ip)) if ip.is_loopback() => ip.into(),
        _ => {
            return Err(invalid(
                "Test media origin must use a literal loopback address",
            ));
        }
    };
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("Test media origin must be loopback HTTP"));
    }
    Ok(Some(SocketAddr::new(
        ip,
        url.port_or_known_default()
            .ok_or_else(|| invalid("Missing test port"))?,
    )))
}
impl MediaProxy {
    pub fn new() -> io::Result<Self> {
        Self::with_fixture(fixture_origin()?)
    }
    pub fn with_fixture(fixture: Option<SocketAddr>) -> io::Result<Self> {
        // Nix's offline sandbox has no resolv.conf. Literal destinations need
        // no resolver; domain lookups fail closed when system DNS is absent.
        let resolver = TokioResolver::builder_tokio()
            .ok()
            .map(|builder| builder.build())
            .transpose()
            .map_err(io::Error::other)?;
        Self::with_resolver(fixture, resolver)
    }
    fn with_resolver(
        fixture: Option<SocketAddr>,
        resolver: Option<TokioResolver>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (stop, mut stopping) = oneshot::channel();
        let error = Arc::new(Mutex::new(String::new()));
        let diagnostic = error.clone();
        // Start this thread before installing the decoder's socket filter:
        // only this relay is permitted to resolve names using system DNS.
        let join=thread::Builder::new().name("public-media-relay".into()).spawn(move ||runtime.block_on(async move {
            let Ok(listener)=tokio::net::TcpListener::from_std(listener) else {return;};
            let budget=Arc::new(Semaphore::new(16));let mut clients=JoinSet::new();
            loop {
                while clients.try_join_next().is_some() {}
                tokio::select! {
                    _=&mut stopping=>break,
                    Some(_)=clients.join_next(),if !clients.is_empty()=>{},
                    incoming=listener.accept()=>{
                        let Ok((mut stream,_))=incoming else {break;};
                        let Ok(permit)=budget.clone().try_acquire_owned() else {continue;};
                         let diagnostic=diagnostic.clone();
                         let resolver=resolver.clone();
                        clients.spawn(async move {
                            let _permit=permit;
                             if let Err(error)=relay(&mut stream,fixture,resolver.as_ref()).await
                                && error.kind()==io::ErrorKind::PermissionDenied {
                                    *diagnostic.lock().unwrap()=error.to_string();
                                    let _=stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                            }
                        });
                    }
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        }))?;
        Ok(Self {
            address,
            fixture,
            error,
            stop: Some(stop),
            join: Some(join),
        })
    }
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }
    pub fn take_error(&self) -> String {
        std::mem::take(&mut self.error.lock().unwrap())
    }
}

async fn destination(
    url: &Url,
    fixture: Option<SocketAddr>,
    resolver: Option<&TokioResolver>,
) -> io::Result<TcpStream> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("Media URL credentials are forbidden"));
    }
    let host = match url.host() {
        Some(Host::Domain(host)) => host.to_owned(),
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => ip.to_string(),
        None => return Err(invalid("Missing media host")),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid("Missing media port"))?;
    // Never use getaddrinfo/spawn_blocking here: a timed-out libc lookup can
    // survive cancellation and keep native code alive during extension unload.
    let addresses: Vec<_> = if let Ok(ip) = host.parse() {
        vec![SocketAddr::new(ip, port)]
    } else {
        let resolver = resolver.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "No system DNS configuration for media host",
            )
        })?;
        timeout(Duration::from_secs(5), resolver.lookup_ip(host))
            .await?
            .map_err(io::Error::other)?
            .iter()
            .take(17)
            .map(|ip| SocketAddr::new(ip, port))
            .collect()
    };
    if addresses.is_empty()
        || addresses.len() > 16
        || addresses
            .iter()
            .any(|address| Some(*address) != fixture && !public_ip(address.ip()))
    {
        return Err(invalid(
            "Media destination resolves to a private/local or reserved address",
        ));
    }
    let mut last = io::Error::new(
        io::ErrorKind::NotConnected,
        "No public media endpoint connected",
    );
    for address in addresses {
        match timeout(Duration::from_secs(3), TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = error,
            Err(error) => last = error.into(),
        }
    }
    Err(last)
}

async fn relay(
    client: &mut TcpStream,
    fixture: Option<SocketAddr>,
    resolver: Option<&TokioResolver>,
) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(4096);
    let (offset, method, target, headers) = timeout(Duration::from_secs(5), async {
        loop {
            if bytes.len() >= 16384 {
                return Err(invalid("Media proxy header budget exceeded"));
            }
            let mut buffer = [0u8; 1024];
            let n = client.read(&mut buffer).await?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            if bytes.len() + n > 16384 {
                return Err(invalid("Media proxy header budget exceeded"));
            }
            bytes.extend_from_slice(&buffer[..n]);
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut request = httparse::Request::new(&mut headers);
            match request
                .parse(&bytes)
                .map_err(|_| invalid("Malformed media proxy request"))?
            {
                httparse::Status::Partial => continue,
                httparse::Status::Complete(offset) => {
                    return Ok((
                        offset,
                        request.method.unwrap_or("").to_owned(),
                        request.path.unwrap_or("").to_owned(),
                        request
                            .headers
                            .iter()
                            .map(|h| (h.name.to_owned(), h.value.to_vec()))
                            .collect::<Vec<_>>(),
                    ));
                }
            }
        }
    })
    .await??;
    let tunnel = method == "CONNECT";
    let url = Url::parse(&if tunnel {
        format!("https://{target}/")
    } else {
        target
    })
    .map_err(|_| invalid("Invalid media proxy URL"))?;
    if tunnel {
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("Invalid CONNECT authority"));
        }
    } else if url.scheme() != "http" || !matches!(method.as_str(), "GET" | "HEAD" | "POST") {
        return Err(invalid(
            "Media relay only accepts HTTP requests and TLS tunnels",
        ));
    }
    let mut server = destination(&url, fixture, resolver).await?;
    if tunnel {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
    } else {
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        let mut header = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("proxy-authorization")
                || name.eq_ignore_ascii_case("proxy-connection")
                || name.eq_ignore_ascii_case("connection")
            {
                continue;
            }
            header.extend_from_slice(name.as_bytes());
            header.extend_from_slice(b": ");
            header.extend_from_slice(&value);
            header.extend_from_slice(b"\r\n");
        }
        header.extend_from_slice(b"Connection: close\r\n\r\n");
        server.write_all(&header).await?;
    }
    server.write_all(&bytes[offset..]).await?;
    tokio::io::copy_bidirectional(client, &mut server).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn missing_system_dns_still_checks_literal_addresses() {
        let proxy = MediaProxy::with_resolver(None, None).unwrap();
        let mut stream = std::net::TcpStream::connect(proxy.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(b"GET http://127.0.0.1:1/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(destination(
                &Url::parse("https://example.com/").unwrap(),
                None,
                None,
            ))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
    #[test]
    fn shutdown_cancels_unresponsive_dns() {
        use hickory_resolver::{
            config::{NameServerConfig, ResolverConfig},
            net::runtime::TokioRuntimeProvider,
        };
        let dns = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        dns.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut server = NameServerConfig::udp(dns.local_addr().unwrap().ip());
        server.connections[0].port = dns.local_addr().unwrap().port();
        let resolver = TokioResolver::builder_with_config(
            ResolverConfig::from_name_servers(vec![server]),
            TokioRuntimeProvider::default(),
        )
        .build()
        .unwrap();
        let proxy = MediaProxy::with_resolver(None, Some(resolver)).unwrap();
        let mut stream = std::net::TcpStream::connect(proxy.address).unwrap();
        stream
            .write_all(b"CONNECT pending.example.:443 HTTP/1.1\r\nHost: pending.example.\r\n\r\n")
            .unwrap();
        let mut packet = [0; 4096];
        // Wait until the resolver is genuinely in flight; deliberately never reply.
        assert!(dns.recv_from(&mut packet).unwrap().0 > 0);
        let start = std::time::Instant::now();
        drop(proxy);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "DNS delayed relay shutdown"
        );
    }
    #[test]
    fn private_destinations_are_denied_before_connect() {
        let proxy = MediaProxy::new().unwrap();
        for request in [
            "GET http://127.0.0.1:1/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "CONNECT [::ffff:127.0.0.1]:443 HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut stream = std::net::TcpStream::connect(proxy.address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        }
    }
}
