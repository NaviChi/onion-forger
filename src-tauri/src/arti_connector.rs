use arti_client::{IsolationToken, StreamPrefs, TorClient};
use http::Uri;
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tor_rtcompat::PreferredRuntime;
use tower_service::Service;

#[derive(Clone)]
pub struct ArtiConnector {
    pub client: TorClient<PreferredRuntime>,
    pub isolation_token: Option<IsolationToken>,
}

impl Service<Uri> for ArtiConnector {
    type Response = ArtiStream;
    type Error = arti_client::Error;
    // type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Uri) -> Self::Future {
        let client = self.client.clone();
        let isolation_token = self.isolation_token.clone();

        let host = req.host().unwrap_or("").to_string();
        let port = req
            .port_u16()
            .unwrap_or(if req.scheme_str() == Some("https") {
                443
            } else {
                80
            });

        Box::pin(async move {
            let mut prefs = StreamPrefs::new();
            if let Some(token) = isolation_token {
                prefs.set_isolation(token);
            }
            let stream = client
                .connect_with_prefs((host.as_str(), port), &prefs)
                .await?;
            Ok(ArtiStream(TokioIo::new(stream)))
        })
    }
}

pub struct ArtiStream(TokioIo<arti_client::DataStream>);

impl Read for ArtiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl Write for ArtiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }
}

impl Connection for ArtiStream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}
