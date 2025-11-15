//! HTTP/HTTPS proxy server with MITM capabilities

use crate::certificate_manager::CertificateManager;
use crate::log_writer::LogWriter;
use crate::proxy_config::ProxyConfig;
use crate::schema::{BodyData, LogEntry, UrlComponents, redact_sensitive_headers};
use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

type BoxBody = http_body_util::combinators::UnsyncBoxBody<Bytes, hyper::Error>;

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed_unsync()
}

pub struct ProxyServer {
    config: ProxyConfig,
    cert_manager: Arc<CertificateManager>,
    log_writer: Arc<LogWriter>,
}

impl ProxyServer {
    pub fn new(config: ProxyConfig, log_writer: Arc<LogWriter>) -> Result<Self> {
        let cert_manager = Arc::new(CertificateManager::new(&config.tls.cert_dir)?);

        Ok(Self {
            config,
            cert_manager,
            log_writer,
        })
    }

    pub async fn run(&self) -> Result<()> {
        let addr = SocketAddr::new(self.config.listen_addr, self.config.listen_port);
        let listener = TcpListener::bind(addr)
            .await
            .context("Failed to bind proxy server")?;

        tracing::info!("Proxy server listening on {}", addr);
        tracing::info!("Set environment variables:");
        tracing::info!("  export HTTP_PROXY=http://{}", addr);
        tracing::info!("  export HTTPS_PROXY=http://{}", addr);

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            tracing::debug!("Accepted connection from {}", peer_addr);

            let config = self.config.clone();
            let cert_manager = self.cert_manager.clone();
            let log_writer = self.log_writer.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, config, cert_manager, log_writer).await {
                    tracing::error!("Connection error: {}", e);
                }
            });
        }
    }

    async fn handle_connection(
        stream: TcpStream,
        config: ProxyConfig,
        cert_manager: Arc<CertificateManager>,
        log_writer: Arc<LogWriter>,
    ) -> Result<()> {
        let io = TokioIo::new(stream);

        let service = service_fn(move |req| {
            Self::proxy_request(
                req,
                config.clone(),
                cert_manager.clone(),
                log_writer.clone(),
            )
        });

        http1::Builder::new()
            .preserve_header_case(true)
            .title_case_headers(true)
            .serve_connection(io, service)
            .with_upgrades()
            .await?;

        Ok(())
    }

    async fn proxy_request(
        req: Request<Incoming>,
        config: ProxyConfig,
        cert_manager: Arc<CertificateManager>,
        log_writer: Arc<LogWriter>,
    ) -> Result<Response<BoxBody>, Box<dyn std::error::Error + Send + Sync>> {
        let method = req.method().clone();
        let uri = req.uri().clone();

        tracing::info!("{} {}", method, uri);

        // Handle CONNECT method for HTTPS
        if method == Method::CONNECT {
            return Self::handle_connect(req, config, cert_manager, log_writer).await;
        }

        // Handle regular HTTP proxy
        Self::handle_http_proxy(req, config, log_writer).await
    }

    async fn handle_connect(
        req: Request<Incoming>,
        config: ProxyConfig,
        cert_manager: Arc<CertificateManager>,
        log_writer: Arc<LogWriter>,
    ) -> Result<Response<BoxBody>, Box<dyn std::error::Error + Send + Sync>> {
        // Extract full authority (hostname:port)
        let authority = req.uri()
            .authority()
            .ok_or_else(|| {
                tracing::error!("CONNECT missing authority");
                anyhow::anyhow!("CONNECT missing authority")
            })?
            .as_str()
            .to_string();

        // Extract hostname for interception filtering
        let hostname = authority.split(':').next().unwrap_or(&authority);

        tracing::info!("CONNECT to {}", authority);

        // Check if this host should be intercepted
        let should_intercept = config.filtering.target_hosts.is_empty()
            || config.filtering.target_hosts.iter().any(|h| hostname.contains(h));

        if should_intercept {
            // MITM mode: intercept and decrypt
            // Extract upgrade future BEFORE moving req into spawned task
            // This is critical for hyper's upgrade mechanism to work correctly
            let upgrade = hyper::upgrade::on(req);

            tokio::task::spawn(async move {
                match upgrade.await {
                    Ok(upgraded) => {
                        tracing::debug!("MITM upgrade successful for {}", authority);
                        if let Err(e) = Self::mitm_tunnel(
                            upgraded,
                            authority.clone(),
                            config,
                            cert_manager,
                            log_writer,
                        )
                        .await
                        {
                            tracing::error!("MITM tunnel error for {}: {}", authority, e);
                        }
                    }
                    Err(e) => tracing::error!("MITM upgrade error for {}: {}", authority, e),
                }
            });

            Ok(Response::new(full("")))
        } else {
            // Passthrough mode: just tunnel
            tracing::debug!("Passthrough mode for {}", authority);

            // Extract upgrade future BEFORE moving req into spawned task
            // This is critical for hyper's upgrade mechanism to work correctly
            let upgrade = hyper::upgrade::on(req);

            tokio::task::spawn(async move {
                match upgrade.await {
                    Ok(upgraded) => {
                        tracing::debug!("Passthrough upgrade successful for {}", authority);
                        if let Err(e) = Self::tunnel(upgraded, authority.clone()).await {
                            tracing::error!("Tunnel error for {}: {}", authority, e);
                        }
                    }
                    Err(e) => tracing::error!("Passthrough upgrade error for {}: {}", authority, e),
                }
            });

            Ok(Response::new(full("")))
        }
    }

    async fn mitm_tunnel(
        upgraded: hyper::upgrade::Upgraded,
        host: String,
        config: ProxyConfig,
        cert_manager: Arc<CertificateManager>,
        log_writer: Arc<LogWriter>,
    ) -> Result<()> {
        let hostname = host.split(':').next().unwrap_or(&host);

        // Get or generate certificate for this host
        let (certs, key) = cert_manager.get_certificate(hostname).await?;

        // Build TLS server config
        let tls_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| anyhow::anyhow!("TLS config error: {}", e))?;

        let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

        // Wrap the upgraded connection with TLS
        let tls_stream = tls_acceptor
            .accept(TokioIo::new(upgraded))
            .await
            .map_err(|e| anyhow::anyhow!("TLS accept error: {}", e))?;

        // Now handle HTTPS traffic
        let io = TokioIo::new(tls_stream);

        let service = service_fn(move |req| {
            Self::handle_https_request(
                req,
                host.clone(),
                config.clone(),
                log_writer.clone(),
            )
        });

        http1::Builder::new()
            .preserve_header_case(true)
            .title_case_headers(true)
            .serve_connection(io, service)
            .await
            .map_err(|e| anyhow::anyhow!("HTTPS serve error: {}", e))?;

        Ok(())
    }

    async fn tunnel(upgraded: hyper::upgrade::Upgraded, host: String) -> Result<()> {
        // Connect to the target
        let target_stream = TcpStream::connect(&host)
            .await
            .context("Failed to connect to target")?;

        // Wrap upgraded connection in TokioIo for AsyncRead/AsyncWrite
        let mut client = TokioIo::new(upgraded);
        let (mut server_read, mut server_write) = target_stream.into_split();

        // Bidirectional copy
        let (mut client_read, mut client_write) = tokio::io::split(&mut client);

        let client_to_server = tokio::io::copy(&mut client_read, &mut server_write);
        let server_to_client = tokio::io::copy(&mut server_read, &mut client_write);

        tokio::try_join!(client_to_server, server_to_client)?;

        Ok(())
    }

    async fn handle_https_request(
        req: Request<Incoming>,
        host: String,
        config: ProxyConfig,
        log_writer: Arc<LogWriter>,
    ) -> Result<Response<BoxBody>, Box<dyn std::error::Error + Send + Sync>> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| q.to_string());

        // Strip default HTTPS port (443) from host if present
        let host_without_default_port = host.strip_suffix(":443").unwrap_or(&host);

        let full_uri = if let Some(q) = query {
            format!("https://{}{}?{}", host_without_default_port, path, q)
        } else {
            format!("https://{}{}", host_without_default_port, path)
        };

        tracing::info!("HTTPS: {} {}", method, full_uri);

        // Forward the request
        Self::forward_request(req, full_uri.parse().unwrap(), config, log_writer).await
    }

    async fn handle_http_proxy(
        req: Request<Incoming>,
        config: ProxyConfig,
        log_writer: Arc<LogWriter>,
    ) -> Result<Response<BoxBody>, Box<dyn std::error::Error + Send + Sync>> {
        let uri = req.uri().clone();
        Self::forward_request(req, uri, config, log_writer).await
    }

    async fn forward_request(
        req: Request<Incoming>,
        uri: Uri,
        config: ProxyConfig,
        log_writer: Arc<LogWriter>,
    ) -> Result<Response<BoxBody>, Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Instant;

        let request_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let correlation_id = session_id.to_string();
        let method = req.method().clone();
        let headers = req.headers().clone();

        // Collect request body
        let (parts, body) = req.into_parts();
        let body_bytes = body
            .collect()
            .await?
            .to_bytes();

        // Log request
        if config.recording.include_bodies {
            Self::log_request(
                &request_id,
                &session_id.to_string(),
                &correlation_id,
                &method,
                &uri,
                &headers,
                &body_bytes,
                &config,
                &log_writer,
            )
            .await;
        }

        // Start timing
        let start = Instant::now();

        // Create HTTPS client
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()?
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        let client: hyper_util::client::legacy::Client<_, Full<Bytes>> =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(https);

        // Rebuild request with collected body
        let mut new_req = Request::builder()
            .method(parts.method)
            .uri(uri.clone());

        for (name, value) in parts.headers.iter() {
            new_req = new_req.header(name, value);
        }

        let new_req = new_req
            .body(Full::new(body_bytes.clone()))?;

        // Send request
        let resp = client.request(new_req).await
            .map_err(|e| {
                tracing::error!("Failed to forward request to {}: {:?}", uri, e);
                e
            })?;

        // Calculate duration
        let duration_ms = start.elapsed().as_millis() as u64;

        let (resp_parts, resp_body) = resp.into_parts();

        // Collect response body
        let resp_body_bytes = resp_body
            .collect()
            .await?
            .to_bytes();

        // Log response
        if config.recording.include_bodies {
            Self::log_response(
                &request_id,
                &session_id.to_string(),
                &correlation_id,
                resp_parts.status,
                &resp_parts.headers,
                &resp_body_bytes,
                duration_ms,
                &config,
                &log_writer,
            )
            .await;
        }

        // Rebuild response
        let mut response = Response::builder().status(resp_parts.status);

        for (name, value) in resp_parts.headers.iter() {
            response = response.header(name, value);
        }

        Ok(response.body(full(resp_body_bytes))?)
    }

    async fn log_request(
        request_id: &Uuid,
        session_id: &str,
        correlation_id: &str,
        method: &Method,
        uri: &Uri,
        headers: &hyper::HeaderMap,
        body: &Bytes,
        config: &ProxyConfig,
        log_writer: &Arc<LogWriter>,
    ) {
        // Extract content encoding and type
        let content_encoding = headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Convert headers to HashMap
        let headers_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // Redact sensitive headers
        let redacted_headers = redact_sensitive_headers(&headers_map);

        // Parse URL components
        let url_components = Self::parse_url_components(uri);

        // Detect API endpoint pattern
        let endpoint_pattern = Self::detect_endpoint_pattern(uri.path());

        // Extract API version
        let api_version = Self::extract_api_version(uri, &headers_map);

        // Generate curl command (using redacted headers)
        let curl_command = Some(Self::generate_curl_command(method, uri, &redacted_headers, body));

        // Process body with intelligent handling
        let body_data = BodyData::from_bytes(
            body,
            content_encoding,
            content_type,
            config.recording.max_body_size,
        );

        let entry = LogEntry::new_proxy_request(
            session_id.to_string(),
            correlation_id.to_string(),
            *request_id,
            method.to_string(),
            uri.to_string(),
            redacted_headers,
            body_data,
            None, // tls_handshake_ms - could be tracked in future
            url_components,
            curl_command,
            endpoint_pattern,
            api_version,
        );

        // Use unified LogWriter with file locking for safe concurrent writes
        let _ = log_writer.write_async(entry).await;
    }

    async fn log_response(
        request_id: &Uuid,
        session_id: &str,
        correlation_id: &str,
        status: StatusCode,
        headers: &hyper::HeaderMap,
        body: &Bytes,
        duration_ms: u64,
        config: &ProxyConfig,
        log_writer: &Arc<LogWriter>,
    ) {
        // Extract content encoding and type
        let content_encoding = headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Convert headers to HashMap
        let headers_map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // Redact sensitive headers (e.g., Set-Cookie)
        let redacted_headers = redact_sensitive_headers(&headers_map);

        // Process body with intelligent handling
        let body_data = BodyData::from_bytes(
            body,
            content_encoding,
            content_type,
            config.recording.max_body_size,
        );

        let entry = LogEntry::new_proxy_response(
            session_id.to_string(),
            correlation_id.to_string(),
            *request_id,
            status.as_u16(),
            redacted_headers,
            body_data,
            duration_ms,
        );

        // Use unified LogWriter with file locking for safe concurrent writes
        let _ = log_writer.write_async(entry).await;
    }

    /// Parse URI into URL components
    fn parse_url_components(uri: &Uri) -> Option<UrlComponents> {
        let scheme = uri.scheme_str().unwrap_or("https").to_string();
        let authority = uri.authority()?;
        let host = authority.host().to_string();
        let port = authority.port_u16();
        let path = uri.path().to_string();

        // Parse query parameters
        let query_params: HashMap<String, String> = uri
            .query()
            .map(|q| {
                q.split('&')
                    .filter_map(|pair| {
                        let mut parts = pair.splitn(2, '=');
                        Some((
                            parts.next()?.to_string(),
                            parts.next().unwrap_or("").to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Some(UrlComponents {
            scheme,
            host,
            port,
            path,
            query_params,
        })
    }

    /// Detect API endpoint pattern from URL path
    fn detect_endpoint_pattern(path: &str) -> Option<String> {
        // Match common API patterns
        if path.contains("/v1/messages") {
            Some("/v1/messages".to_string())
        } else if path.contains("/api/") {
            // Extract pattern up to the first dynamic segment
            let parts: Vec<&str> = path.split('/').collect();
            let mut pattern_parts = Vec::new();
            for part in parts {
                if part.is_empty() {
                    continue;
                }
                pattern_parts.push(part);
                // Stop at obvious IDs or dynamic segments (UUIDs, numbers, etc.)
                if part.len() > 20 || part.chars().all(|c| c.is_numeric() || c == '-') {
                    break;
                }
            }
            if !pattern_parts.is_empty() {
                Some(format!("/{}", pattern_parts.join("/")))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Extract API version from URL or headers
    fn extract_api_version(uri: &Uri, headers: &HashMap<String, String>) -> Option<String> {
        // Try path first
        if let Some(path) = uri.path().split('/').find(|s| s.starts_with('v') && s[1..].chars().all(|c| c.is_numeric() || c == '.')) {
            return Some(path.to_string());
        }

        // Try headers
        if let Some(version) = headers.get("anthropic-version") {
            return Some(version.clone());
        }
        if let Some(version) = headers.get("api-version") {
            return Some(version.clone());
        }

        None
    }

    /// Generate curl command for replaying the request
    fn generate_curl_command(
        method: &Method,
        uri: &Uri,
        headers: &HashMap<String, String>,
        body: &Bytes,
    ) -> String {
        let mut cmd = format!("curl -X {} '{}'", method, uri);

        // Add headers (sensitive ones already redacted in headers map)
        for (key, value) in headers {
            // Skip headers that curl adds automatically
            if key.to_lowercase() == "host" || key.to_lowercase() == "content-length" {
                continue;
            }
            cmd.push_str(&format!(" \\\n  -H '{}: {}'", key, value));
        }

        // Add body if present
        if !body.is_empty() {
            if let Ok(body_str) = std::str::from_utf8(body) {
                // Escape single quotes in JSON
                let escaped_body = body_str.replace('\'', "'\\''");
                cmd.push_str(&format!(" \\\n  -d '{}'", escaped_body));
            } else {
                cmd.push_str(" \\\n  -d '[BINARY DATA]'");
            }
        }

        cmd
    }
}
