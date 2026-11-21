//! URL normalization (pure) and shortener resolution (network).

use crate::net::WebClient;
use std::time::Duration;

use url::Url;

/// Hosts whose links are redirects to the real article.
pub const SHORTENER_HOSTS: &[&str] = &["t.co", "lnkd.in", "bit.ly"];

/// Query parameters that only track the click and never change the page.
/// An entry ending in `*` matches any parameter with that prefix.
pub const TRACKING_PARAMS: &[&str] = &[
    "utm_*", "fbclid", "gclid", "mc_cid", "mc_eid", "ref_src", "igshid",
];

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Rewrites `url` into the form used as the deduplication key.
///
/// - lowercase host
/// - no fragment (`#top`)
/// - no tracking parameters (see [`TRACKING_PARAMS`]); other parameters
///   keep their order; an empty query is removed entirely (no bare `?`)
/// - no trailing `/` on a non-root path (`/a/` → `/a`, but `/` stays)
pub fn normalize_url(url: &Url) -> Url {
    let mut out = url.clone();
    let mut filtered_pairs: Vec<(String, String)> = Default::default();
    for (key, value) in url.query_pairs() {
        if !is_tracking(key.as_ref()) {
            filtered_pairs.push((key.into_owned(), value.into_owned()));
        }
    }
    out.set_fragment(None);
    // now remove the trailing slash if not root
    if out.path().len() > 1 {
        let trimmed = out.path().trim_end_matches('/').to_owned();
        out.set_path(&trimmed);
    }
    if filtered_pairs.is_empty() {
        out.set_query(None);
    } else {
        out.query_pairs_mut().clear().extend_pairs(filtered_pairs);
    }
    out
}

/// Follows a known shortener to its destination. Any other URL, and any
/// failure, returns `url` unchanged.
pub async fn resolve_shortener(client: &WebClient, url: &Url) -> Url {
    resolve_shortener_with_hosts(client, url, SHORTENER_HOSTS).await
}

/// [`resolve_shortener`] with a custom host list, so tests can use a local
/// mock server.
pub async fn resolve_shortener_with_hosts(client: &WebClient, url: &Url, hosts: &[&str]) -> Url {
    if !url.host_str().is_some_and(|h| hosts.contains(&h)) {
        return url.clone();
    }
    // HEAD is cheap, but some shorteners reject it (405), so retry with GET.
    // The client's redirect policy follows the chain; `resp.url()` is where
    // it ended. Unchanged URL means no redirect happened: try the next method.
    for method in [reqwest::Method::HEAD, reqwest::Method::GET] {
        let Ok(request) = client.request(method, url) else {
            tracing::debug!(%url, "shortener URL points at a blocked address");
            return url.clone();
        };
        let sent = request.timeout(RESOLVE_TIMEOUT).send().await;
        match sent {
            Ok(resp) if resp.url() != url => return resp.url().clone(),
            Ok(_) => {}
            Err(err) => tracing::debug!(%url, %err, "shortener request failed"),
        }
    }
    url.clone()
}

/// Resolves a shortener, then normalizes the result.
pub async fn canonicalize(client: &WebClient, url: &Url) -> Url {
    normalize_url(&resolve_shortener(client, url).await)
}

/// Checks if the key is from the tracking slice
fn is_tracking(key: &str) -> bool {
    TRACKING_PARAMS.iter().any(|p| match p.strip_suffix('*') {
        Some(p) => key.starts_with(p),
        _ => key == *p,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String {
        normalize_url(&Url::parse(s).unwrap()).to_string()
    }

    #[test]
    fn strips_tracking_params() {
        assert_eq!(
            norm("https://Ex.com/a/?utm_source=x&id=7#top"),
            "https://ex.com/a?id=7"
        );
    }

    #[test]
    fn keeps_root_slash() {
        assert_eq!(norm("https://ex.com/"), "https://ex.com/");
    }

    #[test]
    fn drops_query_left_empty() {
        assert_eq!(
            norm("https://ex.com/a?utm_medium=x&fbclid=y"),
            "https://ex.com/a"
        );
    }

    #[test]
    fn keeps_other_params_in_order() {
        assert_eq!(
            norm("https://ex.com/a?b=2&gclid=z&a=1"),
            "https://ex.com/a?b=2&a=1"
        );
    }

    #[test]
    fn similar_names_are_not_tracking() {
        // Only exact names (or the `utm_` prefix) count.
        assert_eq!(
            norm("https://ex.com/a?utm=1&fbclid2=2"),
            "https://ex.com/a?utm=1&fbclid2=2"
        );
    }

    #[test]
    fn root_with_port_keeps_slash() {
        assert_eq!(norm("https://ex.com:8080/"), "https://ex.com:8080/");
    }

    #[test]
    fn drops_trailing_slash_on_nested_path() {
        assert_eq!(norm("https://ex.com/a/b/"), "https://ex.com/a/b");
    }
}
