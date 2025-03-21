use crate::errors::*;
use http::Method;
use http::{HeaderMap, HeaderValue};
use once_cell::sync::{Lazy, OnceCell};
use reqwest::redirect::Policy;
use unicase::Ascii;
use url::Url;
use warp::hyper::Body;
use warp::path::FullPath;
use warp::{Rejection, hyper::body::Bytes};
use warp_reverse_proxy::QueryParameters;
use warp_reverse_proxy::Request;
use warp_reverse_proxy::errors::Error as ProxyError;

// these need to be two different reqwest clients because redirect policies are very global
pub static CLIENT_REDIRECT: OnceCell<reqwest::Client> = OnceCell::new();
pub static CLIENT_NOREDIRECT: OnceCell<reqwest::Client> = OnceCell::new();

fn default_redirect_client() -> reqwest::Client {
    reqwest_client(Policy::default())
}

fn default_noredirect_client() -> reqwest::Client {
    reqwest_client(Policy::none())
}

fn reqwest_client(policy: Policy) -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(policy)
        .build()
        .expect("Default reqwest client couldn't build")
}

pub async fn download(
    method: Method,
    url: Url,
    headers: Option<HeaderMap>,
) -> Result<reqwest::Response> {
    send_req(method, url, headers, true).await
}

pub async fn send_req(
    method: Method,
    url: Url,
    headers: Option<HeaderMap>,
    follow_redirects: bool,
) -> Result<reqwest::Response> {
    debug!("Sending request to {:?}", url.to_string());
    let client = if follow_redirects {
        CLIENT_REDIRECT.get_or_init(default_redirect_client)
    } else {
        CLIENT_NOREDIRECT.get_or_init(default_noredirect_client)
    };
    let mut request = client.request(method, url);
    if let Some(headers) = headers {
        request = request.headers(headers);
    }
    let response = request.send().await?;

    trace!("Upstream http response: {:?}", response);
    debug!(
        "Upstream response: {:?} {:?} - bytes={:?} etag={:?}",
        response.url().to_string(),
        response.status(),
        response.headers().get("content-length"),
        response.headers().get("etag")
    );

    Ok(response)
}

pub async fn proxy_to_and_forward_response(
    proxy_uri: String,
    original_uri: FullPath,
    params: QueryParameters,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<http::Response<Body>, Rejection> {
    let request =
        filtered_data_to_request(proxy_uri, (original_uri, params, method, headers, body))
            .map_err(warp::reject::custom)?;
    let response = proxy_request(request).await.map_err(warp::reject::custom)?;
    response_to_reply(response)
        .await
        .map_err(warp::reject::custom)
}

fn filtered_data_to_request(
    proxy_address: String,
    request: Request,
) -> Result<reqwest::Request, ProxyError> {
    let (_uri, params, method, headers, body) = request;

    let proxy_uri = if let Some(params) = params {
        format!("{proxy_address}?{params}")
    } else {
        proxy_address
    };

    let headers = remove_hop_headers(&headers);

    CLIENT_NOREDIRECT
        .get_or_init(default_noredirect_client)
        .request(method, proxy_uri)
        .headers(headers)
        .body(body)
        .build()
        .map_err(ProxyError::Request)
}

fn is_hop_header(header_name: &str) -> bool {
    static HOP_HEADERS: Lazy<Vec<Ascii<&'static str>>> = Lazy::new(|| {
        vec![
            Ascii::new("Connection"),
            Ascii::new("Keep-Alive"),
            Ascii::new("Proxy-Authenticate"),
            Ascii::new("Proxy-Authorization"),
            Ascii::new("Te"),
            Ascii::new("Trailers"),
            Ascii::new("Transfer-Encoding"),
            Ascii::new("Upgrade"),
        ]
    });

    HOP_HEADERS.iter().any(|h| h == &header_name)
}

fn remove_hop_headers(headers: &HeaderMap<HeaderValue>) -> HeaderMap<HeaderValue> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            if !is_hop_header(k.as_str()) {
                Some((k.clone(), v.clone()))
            } else {
                None
            }
        })
        .collect()
}

async fn proxy_request(request: reqwest::Request) -> Result<reqwest::Response, ProxyError> {
    CLIENT_NOREDIRECT
        .get_or_init(default_noredirect_client)
        .execute(request)
        .await
        .map_err(ProxyError::Request)
}

async fn response_to_reply(
    response: reqwest::Response,
) -> Result<http::Response<Body>, ProxyError> {
    let mut builder = http::Response::builder();
    for (k, v) in remove_hop_headers(response.headers()).iter() {
        builder = builder.header(k, v);
    }
    let status = response.status();
    let body = Body::wrap_stream(response.bytes_stream());
    builder.status(status).body(body).map_err(ProxyError::Http)
}
