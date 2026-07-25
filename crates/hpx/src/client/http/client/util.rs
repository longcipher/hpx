use http::{
    Request, Uri,
    uri::{Authority, PathAndQuery, Scheme},
};

use super::error::{Error, ErrorKind};

#[expect(
    clippy::expect_used,
    reason = "parts only contains a cloned path_and_query, which is always a valid Uri"
)]
pub(super) fn origin_form(uri: &mut Uri) {
    let path = match uri.path_and_query() {
        Some(path) if path.as_str() != "/" => {
            let mut parts = ::http::uri::Parts::default();
            parts.path_and_query.replace(path.clone());
            Uri::from_parts(parts).expect("path is valid uri")
        }
        _none_or_just_slash => {
            debug_assert_eq!(Uri::default(), "/");
            Uri::default()
        }
    };
    *uri = path;
}

pub(super) fn absolute_form(uri: &Uri) {
    debug_assert!(uri.scheme().is_some(), "absolute_form needs a scheme");
    debug_assert!(
        uri.authority().is_some(),
        "absolute_form needs an authority"
    );
}

#[expect(
    clippy::expect_used,
    reason = "parts only contains a cloned authority, which is always a valid Uri"
)]
pub(super) fn authority_form(uri: &mut Uri) {
    if let Some(path) = uri.path_and_query() {
        // `https://hyper.rs` would parse with `/` path, don't
        // annoy people about that...
        if path != "/" {
            warn!("HTTP/1.1 CONNECT request stripping path: {:?}", path);
        }
    }
    *uri = match uri.authority() {
        Some(auth) => {
            let mut parts = ::http::uri::Parts::default();
            parts.authority = Some(auth.clone());
            Uri::from_parts(parts).expect("authority is valid")
        }
        None => {
            unreachable!("authority_form with relative uri");
        }
    };
}

#[expect(
    clippy::expect_used,
    reason = "scheme+authority+static '/' path always builds a valid base URI"
)]
pub(super) fn normalize_uri<B>(req: &mut Request<B>, is_http_connect: bool) -> Result<Uri, Error> {
    let uri = req.uri().clone();

    let build_base_uri = |scheme: Scheme, authority: Authority| {
        Uri::builder()
            .scheme(scheme)
            .authority(authority)
            .path_and_query(PathAndQuery::from_static("/"))
            .build()
            .expect("valid base URI")
    };

    match (uri.scheme(), uri.authority()) {
        (Some(scheme), Some(auth)) => Ok(build_base_uri(scheme.clone(), auth.clone())),
        (None, Some(auth)) if is_http_connect => {
            let scheme = match auth.port_u16() {
                Some(443) => Scheme::HTTPS,
                _ => Scheme::HTTP,
            };
            set_scheme(req.uri_mut(), scheme.clone());
            Ok(build_base_uri(scheme, auth.clone()))
        }
        _ => {
            debug!("Client requires absolute-form URIs, received: {:?}", uri);
            Err(Error::new_kind(ErrorKind::UserAbsoluteUriRequired))
        }
    }
}

pub(super) fn get_non_default_port(uri: &Uri) -> Option<http::uri::Port<&str>> {
    match (uri.port().map(|p| p.as_u16()), is_schema_secure(uri)) {
        (Some(443), true) => None,
        (Some(80), false) => None,
        _ => uri.port(),
    }
}

#[expect(
    clippy::expect_used,
    reason = "existing Uri parts plus a scheme and static '/' path always forms a valid Uri"
)]
fn set_scheme(uri: &mut Uri, scheme: Scheme) {
    debug_assert!(
        uri.scheme().is_none(),
        "set_scheme expects no existing scheme"
    );
    let old = std::mem::take(uri);
    let mut parts: ::http::uri::Parts = old.into();
    parts.scheme = Some(scheme);
    parts.path_and_query = Some("/".parse().expect("slash is a valid path"));
    *uri = Uri::from_parts(parts).expect("scheme is valid");
}

fn is_schema_secure(uri: &Uri) -> bool {
    uri.scheme_str()
        .is_some_and(|scheme_str| matches!(scheme_str, "wss" | "https"))
}
