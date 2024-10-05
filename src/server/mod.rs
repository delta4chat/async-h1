//! Process HTTP connections on the server.

use async_io::Timer;
use futures_lite::io::{self, AsyncRead as Read, AsyncWrite as Write};
use futures_lite::prelude::*;
/*=======
use async_std::future::{timeout, Future, TimeoutError};
use async_std::io::{self, Read, Write};
>>>>>>> origin/v3*/

use http_types::upgrade::Connection;
use http_types::{
    headers::{CONNECTION, UPGRADE},
    Version,
};
use http_types::{Request, Response, StatusCode};
use std::{future::Future, marker::PhantomData, time::Duration};
mod body_reader;
mod decode;
mod encode;

pub use decode::decode;
pub use encode::Encoder;

/// Configure the server.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Timeout to handle headers. Defaults to 60s.
    headers_timeout: Option<Duration>,
    default_host: Option<String>,
}

impl ServerOptions {
    /// constructs a new ServerOptions with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// sets the timeout by which the headers must have been received
    pub fn with_headers_timeout(mut self, headers_timeout: Duration) -> Self {
        self.headers_timeout = Some(headers_timeout);
        self
    }

    /// Sets the default http 1.0 host for this server. If no host
    /// header is provided on an http/1.0 request, this host will be
    /// used to construct the Request Url.
    ///
    /// If this is not provided, the server will respond to all
    /// http/1.0 requests with status `505 http version not
    /// supported`, whether or not a host header is provided.
    ///
    /// The default value for this is None, and as a result async-h1
    /// is by default an http-1.1-only server.
    pub fn with_default_host(mut self, default_host: &str) -> Self {
        self.default_host = Some(default_host.into());
        self
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            headers_timeout: Some(Duration::from_secs(60)),
            default_host: None,
        }
    }
}

/// Accept a new incoming HTTP/1.1 connection.
///
/// Supports `KeepAlive` requests by default.
pub async fn accept<RW, F, Fut>(io: RW, endpoint: F) -> crate::Result<()>
where
    RW: Read + Write + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    Server::new(io, endpoint).accept().await
}

/// Accept a new incoming HTTP/1.1 connection.
///
/// Supports `KeepAlive` requests by default.
pub async fn accept_with_opts<RW, F, Fut>(
    io: RW,
    endpoint: F,
    opts: ServerOptions,
) -> crate::Result<()>
where
    RW: Read + Write + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    Server::new(io, endpoint).with_opts(opts).accept().await
}

/// struct for server
#[derive(Debug)]
pub struct Server<RW, F, Fut> {
    io: RW,
    endpoint: F,
    opts: ServerOptions,
    _phantom: PhantomData<Fut>,
}

/// An enum that represents whether the server should accept a subsequent request
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ConnectionStatus {
    /// The server should not accept another request
    Close,

    /// The server may accept another request
    KeepAlive,
}

impl<RW, F, Fut> Server<RW, F, Fut>
where
    RW: Read + Write + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    /// builds a new server
    pub fn new(io: RW, endpoint: F) -> Self {
        Self {
            io,
            endpoint,
            opts: Default::default(),
            _phantom: PhantomData,
        }
    }

    /// with opts
    pub fn with_opts(mut self, opts: ServerOptions) -> Self {
        self.opts = opts;
        self
    }

    /// accept in a loop
    pub async fn accept(&mut self) -> crate::Result<()> {
        while ConnectionStatus::KeepAlive == self.accept_one().await? {}
        Ok(())
    }

    /// accept one request
    pub async fn accept_one(&mut self) -> crate::Result<ConnectionStatus>
    where
        RW: Read + Write + Clone + Send + Sync + Unpin + 'static,
        F: Fn(Request) -> Fut,
        Fut: Future<Output = Response>,
    {
        // Decode a new request, timing out if this takes longer than the timeout duration.
        let fut = decode(self.io.clone(), &self.opts);

        let (req, mut body) = if let Some(timeout_duration) = self.opts.headers_timeout {
            match fut
                .or(async {
                    Timer::after(timeout_duration).await;
                    Ok(None)
                })
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => return Ok(ConnectionStatus::Close), /* EOF or timeout */
                Err(e) => return Err(e),
            }
        } else {
            match fut.await? {
                Some(r) => r,
                None => return Ok(ConnectionStatus::Close), /* EOF */
            }
        };

        let req_version = req.version();


        let connection_header =
            req.header(CONNECTION)
            .map(|connection| connection.as_str())
            .unwrap_or("")
            .to_string();

        let res_header_keepalive = {
            let c = connection_header.to_ascii_lowercase();
            if c == "keep-alive" || c.contains("keep-alive,") {
                "keep-alive"
            } else if c == "close" || c.contains("close") {
                "close"
            } else {
                match req_version {
                    Some(Version::Http1_1) => "keep-alive",
                    Some(Version::Http1_0) => "close",
                    _ => { unreachable!(); }
                }
            }
        };

        let close_connection =
            match res_header_keepalive {
                "close" => true,
                _ => false
            };
        /*
        let mut close_connection =
            if req_version == Some(Version::Http1_0) {
                ! connection_header.eq_ignore_ascii_case("keep-alive")
            } else {
                connection_header.eq_ignore_ascii_case("close")
            };
        */


        let connection_header_is_upgrade = connection_header.split(',').any(|s| s.trim().eq_ignore_ascii_case("upgrade"));
        let has_upgrade_header = req.header(UPGRADE).is_some();
        let upgrade_requested = has_upgrade_header && connection_header_is_upgrade;

        let method = req.method();

        // Pass the request to the endpoint and encode the response.
        let mut response = (self.endpoint)(req).await;
        response.set_version(req_version);

        /*
        close_connection |=
            response.header(CONNECTION)
            .map(|c| c.as_str().eq_ignore_ascii_case("close"))
            .unwrap_or(false);
        */

        let upgrade_provided =
            response.status() == StatusCode::SwitchingProtocols && response.has_upgrade();

        if ! upgrade_provided {
            if let Some(hc) = response.header(CONNECTION) {
                let tmp: Vec<_> = hc.iter().collect();
                if tmp.len() != 1 {
                    // is multi "Connection" headers can be properly handled by clients?
                    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#syntax
                    return Err(crate::Error::UnexpectedHeader("should not have multi 'Connection' header"));
                }

                let mut new_hc = hc.last().as_str().to_string();
                if new_hc.is_empty() {
                    new_hc = res_header_keepalive.to_string();
                } else {
                    new_hc.push(',');
                    new_hc.push(' ');
                    new_hc.extend(res_header_keepalive.chars());
                }
                response.insert_header(CONNECTION, new_hc);
            } else {
                response.insert_header(CONNECTION, res_header_keepalive);
            }
        }

        let upgrade_sender = if upgrade_requested && upgrade_provided {
            Some(response.send_upgrade())
        } else {
            None
        };

        let mut encoder = Encoder::new(response, method);

        let bytes_written = io::copy(&mut encoder, &mut self.io).await?;
        log::trace!("wrote {} response bytes", bytes_written);

        let body_bytes_discarded = io::copy(&mut body, &mut io::sink()).await?;
        log::trace!(
            "discarded {} unread request body bytes",
            body_bytes_discarded
        );

        if let Some(upgrade_sender) = upgrade_sender {
            upgrade_sender.send(Connection::new(self.io.clone())).await;
            Ok(ConnectionStatus::Close)
        } else if close_connection {
            Ok(ConnectionStatus::Close)
        } else {
            Ok(ConnectionStatus::KeepAlive)
        }
    }
}
