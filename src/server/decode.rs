//! Process HTTP connections on the server.

use async_dup::{Arc, Mutex};
use futures_lite::io::{AsyncRead as Read, AsyncWrite as Write, BufReader};
use futures_lite::prelude::*;
use http_types::content::ContentLength;
use http_types::headers::{EXPECT, TRANSFER_ENCODING};
//use http_types::{ensure, ensure_eq,format_err};
use http_types::{Body, /*Method,*/ Request, Url, Version};

use crate::{Error, Result}; //

use super::body_reader::BodyReader;
use crate::read_notifier::ReadNotifier;
use crate::{chunked::ChunkedDecoder, ServerOptions};
use crate::{MAX_HEADERS, MAX_HEAD_LENGTH};

const LF: u8 = b'\n';
const SPACE: u8 = b' ';

const CONTINUE_HEADER_VALUE: &str = "100-continue";
const CONTINUE_RESPONSE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

/// Decode an HTTP request on the server.

pub async fn decode<IO>(
    mut io: IO,
    opts: &ServerOptions,
) -> Result<Option<(Request, BodyReader<IO>)>>
where
    IO: Read + Write + Clone + Send + Sync + Unpin + 'static,
{
    let mut reader = BufReader::with_capacity(MAX_HEAD_LENGTH, io.clone()); // Prevent CWE-400 DoS with large HTTP Headers but without LF char.
    let mut buf = Vec::new();
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut httparse_req = httparse::Request::new(&mut headers);

    let mut first_line = true;
    // Keep reading bytes from the stream until we hit the end of the stream.
    loop {
        let bytes_read = reader.read_until(LF, &mut buf).await?;

        // No more bytes are yielded from the stream.
        if bytes_read == 0 {
            return Ok(None);
        }

        // Prevent CWE-400 DoS with large HTTP Headers.
        if buf.len() >= MAX_HEAD_LENGTH {
            return Err(Error::HeadersTooLong);
        }

        if first_line {
            first_line = false;

            let mut split = buf.split(|b| { b == &SPACE });
            let method = split.next().ok_or(Error::MissingMethod)?;

            let path = split.next().ok_or(Error::RequestPathMissing)?;
            let path = non_ascii_printable_to_percent_encoded(path);

            let mut parts = vec![method, &path];
            for part in split {
                parts.push(part);
            }

            buf = parts.join(&SPACE);
        }

        // We've hit the end delimiter of the stream.
        if buf.ends_with(b"\r\n\r\n") || buf.ends_with(b"\n\n") {
            break;
        }
    }

    // Convert our header buf into an httparse instance, and validate.
    let status = httparse_req.parse(&buf)?;

    if status.is_partial() {
        return Err(Error::PartialHead);
    }

    // Convert httparse headers + body into a `http_types::Request` type.
    let method = httparse_req
        .method
        .ok_or(Error::MissingMethod)?
        .parse()
        .map_err(|_| Error::UnrecognizedMethod(httparse_req.method.unwrap().to_string()))?;

    let version = match (&opts.default_host, httparse_req.version) {
        (Some(_), None) | (Some(_), Some(0)) => Version::Http1_0,
        (_, Some(1)) => Version::Http1_1,
        (None, Some(0)) | (None, None) => return Err(Error::HostHeaderMissing),
        (_, Some(other_version)) => return Err(Error::UnsupportedVersion(other_version)),
    };

    let url = url_from_httparse_req(&httparse_req, opts.default_host.as_deref())?;

    let mut req = Request::new(method, url);

    req.set_version(Some(version));

    for header in httparse_req.headers.iter() {
        req.append_header(header.name, std::str::from_utf8(header.value)?);
    }

    let content_length =
        ContentLength::from_headers(&req).map_err(|_| Error::MalformedHeader("content-length"))?;
    let transfer_encoding = req.header(TRANSFER_ENCODING);

    if content_length.is_some() && transfer_encoding.is_some() {
        return Err(Error::UnexpectedHeader("content-length"));
    }

    // Establish a channel to wait for the body to be read. This
    // allows us to avoid sending 100-continue in situations that
    // respond without reading the body, saving clients from uploading
    // their body.
    let (body_read_sender, body_read_receiver) = async_channel::bounded(1);

    if Some(CONTINUE_HEADER_VALUE) == req.header(EXPECT).map(|h| h.as_str()) {
        smolscale2::spawn(async move {
            // If the client expects a 100-continue header, spawn a
            // task to wait for the first read attempt on the body.
            if let Ok(()) = body_read_receiver.recv().await {
                io.write_all(CONTINUE_RESPONSE).await.ok();
            };
            // Since the sender is moved into the Body, this task will
            // finish when the client disconnects, whether or not
            // 100-continue was sent.
        })
        .detach();
    }

    // Check for Transfer-Encoding
    if transfer_encoding
        .map(|te| te.as_str().eq_ignore_ascii_case("chunked"))
        .unwrap_or(false)
    {
        let trailer_sender = req.send_trailers();
        let reader = ChunkedDecoder::new(reader, trailer_sender);
        let reader = Arc::new(Mutex::new(reader));
        let reader_clone = reader.clone();
        let reader = ReadNotifier::new(reader, body_read_sender);
        let reader = BufReader::new(reader);
        req.set_body(Body::from_reader(reader, None));
        Ok(Some((req, BodyReader::Chunked(reader_clone))))
    } else if let Some(content_length) = content_length {
        let len = content_length.len();
        let reader = Arc::new(Mutex::new(reader.take(len)));
        req.set_body(Body::from_reader(
            BufReader::new(ReadNotifier::new(reader.clone(), body_read_sender)),
            Some(len as usize),
        ));
        Ok(Some((req, BodyReader::Fixed(reader))))
    } else {
        Ok(Some((req, BodyReader::None)))
    }
}

fn non_ascii_printable_to_percent_encoded(path: &[u8]) -> Vec<u8> {
    // python: [chr(i) for i in range(256) if chr(i).isascii() and chr(i).isprintable()]
    const WHITELIST: &[u8] = b" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

    let mut out = Vec::new();
    for byte in path.iter() {
        if WHITELIST.contains(byte) {
            out.push(*byte);
        } else {
            out.extend(format!("%{byte:02X}").as_bytes());
        }
    }
    out
}

fn url_from_httparse_req(
    req: &httparse::Request<'_, '_>,
    default_host: Option<&str>,
) -> Result<Url> {
    let path = req.path.ok_or(Error::RequestPathMissing)?;

    let host = req
        .headers
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case("host"));

    let host = match host {
        Some(header) => std::str::from_utf8(header.value)?,
        None => default_host.ok_or(Error::HostHeaderMissing)?,
    };

    if path.starts_with("http://") || path.starts_with("https://") {
        Ok(Url::parse(&path)?)
    } else if path.starts_with('/') {
        Ok(Url::parse(&format!("http://{}{}", host, &path))?)
    } else if req.method.unwrap().eq_ignore_ascii_case("connect") {
        Ok(Url::parse(&format!("http://{}/", &path))?)
    } else {
        Err(Error::UnexpectedURIFormat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn httparse_req(buf: &str, f: impl Fn(httparse::Request<'_, '_>)) {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut res = httparse::Request::new(&mut headers[..]);
        res.parse(buf.as_bytes()).unwrap();
        f(res)
    }

    #[test]
    fn url_for_connect() {
        httparse_req(
            "CONNECT server.example.com:443 HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(url.as_str(), "http://server.example.com:443/");
            },
        );
    }

    #[test]
    fn url_for_host_plus_path() {
        httparse_req(
            "GET /some/resource HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(url.as_str(), "http://server.example.com:443/some/resource");
            },
        )
    }

    #[test]
    fn url_for_host_plus_absolute_url() {
        httparse_req(
            "GET http://domain.com/some/resource HTTP/1.1\r\nHost: server.example.com\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(url.as_str(), "http://domain.com/some/resource"); // host header MUST be ignored according to spec
            },
        )
    }

    #[test]
    fn url_for_conflicting_connect() {
        httparse_req(
            "CONNECT server.example.com:443 HTTP/1.1\r\nHost: conflicting.host\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(url.as_str(), "http://server.example.com:443/");
            },
        )
    }

    #[test]
    fn url_for_malformed_resource_path() {
        httparse_req(
            "GET not-a-url HTTP/1.1\r\nHost: server.example.com\r\n",
            |req| {
                assert!(matches!(
                    url_from_httparse_req(&req, None),
                    Err(Error::UnexpectedURIFormat)
                ));
            },
        )
    }

    #[test]
    fn url_for_double_slash_path() {
        httparse_req(
            "GET //double/slashes HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(
                    url.as_str(),
                    "http://server.example.com:443//double/slashes"
                );
            },
        )
    }
    #[test]
    fn url_for_triple_slash_path() {
        httparse_req(
            "GET ///triple/slashes HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(
                    url.as_str(),
                    "http://server.example.com:443///triple/slashes"
                );
            },
        )
    }

    #[test]
    fn url_for_query() {
        httparse_req(
            "GET /foo?bar=1 HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(url.as_str(), "http://server.example.com:443/foo?bar=1");
            },
        )
    }

    #[test]
    fn url_for_anchor() {
        httparse_req(
            "GET /foo?bar=1#anchor HTTP/1.1\r\nHost: server.example.com:443\r\n",
            |req| {
                let url = url_from_httparse_req(&req, None).unwrap();
                assert_eq!(
                    url.as_str(),
                    "http://server.example.com:443/foo?bar=1#anchor"
                );
            },
        )
    }
}
