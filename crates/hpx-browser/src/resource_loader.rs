use std::collections::HashSet;

use ahash::AHashSet;

use crate::dom::{Dom, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceUrl {
    pub resource_type: ResourceType,
    pub url: String,
}

fn get_attr<'a>(attrs: &'a [crate::dom::Attribute], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.local.eq_ignore_ascii_case(name))
        .map(|a| a.value.as_str())
}

fn extract_from_element(dom: &Dom, id: NodeId) -> Option<ResourceUrl> {
    let node = dom.get(id)?;
    let elem = node.as_element()?;
    let tag = elem.name.local.as_str();
    match tag {
        "link" => {
            let rel = get_attr(&elem.attrs, "rel").unwrap_or("");
            let href = get_attr(&elem.attrs, "href")?;
            if rel.eq_ignore_ascii_case("stylesheet") {
                Some(ResourceUrl {
                    resource_type: ResourceType::Stylesheet,
                    url: href.to_owned(),
                })
            } else if rel.eq_ignore_ascii_case("preload") {
                let as_type = get_attr(&elem.attrs, "as").unwrap_or("");
                let rt = if as_type.eq_ignore_ascii_case("font") {
                    ResourceType::Font
                } else if as_type.eq_ignore_ascii_case("style") {
                    ResourceType::Stylesheet
                } else if as_type.eq_ignore_ascii_case("script") {
                    ResourceType::Script
                } else if as_type.eq_ignore_ascii_case("image") {
                    ResourceType::Image
                } else {
                    return None;
                };
                Some(ResourceUrl {
                    resource_type: rt,
                    url: href.to_owned(),
                })
            } else {
                None
            }
        }
        "script" => {
            let src = get_attr(&elem.attrs, "src")?;
            Some(ResourceUrl {
                resource_type: ResourceType::Script,
                url: src.to_owned(),
            })
        }
        "img" => {
            let src = get_attr(&elem.attrs, "src")?;
            Some(ResourceUrl {
                resource_type: ResourceType::Image,
                url: src.to_owned(),
            })
        }
        "video" | "audio" => {
            let src = get_attr(&elem.attrs, "src")?;
            Some(ResourceUrl {
                resource_type: ResourceType::Media,
                url: src.to_owned(),
            })
        }
        _ => None,
    }
}

/// Extract all resource URLs from the DOM.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn extract_resource_urls(dom: &Dom) -> Vec<ResourceUrl> {
    let mut results = Vec::new();
    let mut seen = AHashSet::new();
    let mut stack = vec![NodeId::DOCUMENT];
    let mut visited = AHashSet::new();

    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(resource) = extract_from_element(dom, id) {
            if seen.insert(resource.url.clone()) {
                results.push(resource);
            }
        }
        // push children in reverse so we visit in document order
        let children = dom.children(id);
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    results
}

/// A fetched resource with its content and metadata.
#[derive(Debug, Clone)]
pub struct LoadedResource {
    pub url: String,
    pub resource_type: ResourceType,
    pub content: String,
    pub content_type: Option<String>,
}

/// Fetch all resources concurrently.
///
/// Respects `block_types` — blocked resources are skipped.
/// Caps concurrent requests at `max_concurrent` (default 6).
/// Individual resource timeout: 5s. Total timeout: 15s.
pub async fn fetch_resources(
    urls: Vec<ResourceUrl>,
    block_types: &HashSet<ResourceType>,
    max_concurrent: usize,
) -> Vec<LoadedResource> {
    let filtered = filter_by_block_types(urls, block_types);
    if filtered.is_empty() {
        return Vec::new();
    }

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let client = hpx::Client::new();

    let total = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut set = tokio::task::JoinSet::new();

        for resource in filtered {
            let sem = semaphore.clone();
            let client = client.clone();
            set.spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return None,
                };
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.get(&resource.url).send(),
                )
                .await;

                match result {
                    Ok(Ok(resp)) => {
                        let content_type = resp
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());
                        let url = resp.uri().to_string();
                        match resp.text().await {
                            Ok(text) => Some(LoadedResource {
                                url,
                                resource_type: resource.resource_type,
                                content: text,
                                content_type,
                            }),
                            Err(_) => None,
                        }
                    }
                    _ => None,
                }
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(Some(loaded)) = res {
                results.push(loaded);
            }
        }
        results
    })
    .await;

    total.unwrap_or_default()
}

/// Filter resources by blocked types.
pub fn filter_by_block_types(
    resources: Vec<ResourceUrl>,
    block_types: &HashSet<ResourceType>,
) -> Vec<ResourceUrl> {
    resources
        .into_iter()
        .filter(|r| !block_types.contains(&r.resource_type))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html_parser::parse_html;

    #[test]
    fn extract_stylesheet() {
        let dom = parse_html(
            r#"<html><head><link rel="stylesheet" href="/style.css"></head><body></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].resource_type, ResourceType::Stylesheet);
        assert_eq!(urls[0].url, "/style.css");
    }

    #[test]
    fn extract_script() {
        let dom =
            parse_html(r#"<html><head><script src="/app.js"></script></head><body></body></html>"#);
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].resource_type, ResourceType::Script);
        assert_eq!(urls[0].url, "/app.js");
    }

    #[test]
    fn extract_img() {
        let dom = parse_html(r#"<html><body><img src="/logo.png"></body></html>"#);
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].resource_type, ResourceType::Image);
        assert_eq!(urls[0].url, "/logo.png");
    }

    #[test]
    fn extract_video_and_audio() {
        let dom = parse_html(
            r#"<html><body><video src="/v.mp4"></video><audio src="/a.mp3"></audio></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().all(|r| r.resource_type == ResourceType::Media));
    }

    #[test]
    fn extract_preload_font() {
        let dom = parse_html(
            r#"<html><head><link rel="preload" href="/font.woff2" as="font" crossorigin></head><body></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].resource_type, ResourceType::Font);
        assert_eq!(urls[0].url, "/font.woff2");
    }

    #[test]
    fn inline_script_ignored() {
        let dom = parse_html(r#"<html><head><script>alert(1)</script></head><body></body></html>"#);
        let urls = extract_resource_urls(&dom);
        assert!(urls.is_empty());
    }

    #[test]
    fn all_resource_types() {
        let dom = parse_html(
            r#"<html>
            <head>
                <link rel="stylesheet" href="/style.css">
                <script src="/app.js"></script>
                <link rel="preload" href="/font.woff2" as="font">
            </head>
            <body>
                <img src="/photo.jpg">
                <video src="/clip.mp4"></video>
            </body>
            </html>"#,
        );
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 5);
        let types: HashSet<ResourceType> = urls.iter().map(|r| r.resource_type).collect();
        assert!(types.contains(&ResourceType::Stylesheet));
        assert!(types.contains(&ResourceType::Script));
        assert!(types.contains(&ResourceType::Image));
        assert!(types.contains(&ResourceType::Font));
        assert!(types.contains(&ResourceType::Media));
    }

    #[test]
    fn dedup_same_url() {
        let dom = parse_html(
            r#"<html><head>
                <link rel="stylesheet" href="/style.css">
                <link rel="stylesheet" href="/style.css">
            </head><body></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn filter_blocks_stylesheets() {
        let dom = parse_html(
            r#"<html><head>
                <link rel="stylesheet" href="/style.css">
                <script src="/app.js"></script>
            </head><body><img src="/logo.png"></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        let mut block = HashSet::new();
        block.insert(ResourceType::Stylesheet);
        let filtered = filter_by_block_types(urls, &block);
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|r| r.resource_type != ResourceType::Stylesheet)
        );
    }

    #[test]
    fn filter_empty_block_returns_all() {
        let dom = parse_html(
            r#"<html><head><link rel="stylesheet" href="/s.css"></head><body><img src="/i.png"></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        let block = HashSet::new();
        let filtered = filter_by_block_types(urls.clone(), &block);
        assert_eq!(filtered.len(), urls.len());
    }

    #[test]
    fn filter_all_blocks_returns_empty() {
        let dom = parse_html(
            r#"<html><head>
                <link rel="stylesheet" href="/style.css">
                <script src="/app.js"></script>
            </head><body><img src="/logo.png"></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        let mut block = HashSet::new();
        block.insert(ResourceType::Stylesheet);
        block.insert(ResourceType::Script);
        block.insert(ResourceType::Image);
        block.insert(ResourceType::Font);
        block.insert(ResourceType::Media);
        let filtered = filter_by_block_types(urls, &block);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_idempotent() {
        let dom = parse_html(
            r#"<html><head>
                <link rel="stylesheet" href="/style.css">
                <script src="/app.js"></script>
            </head><body><img src="/logo.png"></body></html>"#,
        );
        let urls = extract_resource_urls(&dom);
        let mut block = HashSet::new();
        block.insert(ResourceType::Stylesheet);
        let once = filter_by_block_types(urls.clone(), &block);
        let twice = filter_by_block_types(once.clone(), &block);
        assert_eq!(once, twice);
    }

    #[test]
    fn block_images_from_html() {
        let dom =
            parse_html(r#"<html><body><img src="/photo.jpg"><img src="/icon.png"></body></html>"#);
        let urls = extract_resource_urls(&dom);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().all(|r| r.resource_type == ResourceType::Image));
        let mut block = HashSet::new();
        block.insert(ResourceType::Image);
        let filtered = filter_by_block_types(urls, &block);
        assert!(filtered.is_empty());
    }

    #[test]
    fn no_resources() {
        let dom = parse_html(r#"<html><body><p>Hello</p></body></html>"#);
        let urls = extract_resource_urls(&dom);
        assert!(urls.is_empty());
    }

    #[tokio::test]
    async fn fetch_resources_concurrent() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");

        let resources = vec![
            ("/style.css", "text/css", "body{}"),
            ("/app.js", "application/javascript", "alert(1)"),
            ("/logo.png", "image/png", "PNGDATA"),
        ];

        let server_resources = resources.clone();
        let server = tokio::spawn(async move {
            for (path, ct, body) in &server_resources {
                let (mut stream, _) = listener.accept().await.unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let urls: Vec<ResourceUrl> = resources
            .iter()
            .map(|(path, _, _)| ResourceUrl {
                resource_type: if path.ends_with(".css") {
                    ResourceType::Stylesheet
                } else if path.ends_with(".js") {
                    ResourceType::Script
                } else {
                    ResourceType::Image
                },
                url: format!("{base}{path}"),
            })
            .collect();

        let loaded = fetch_resources(urls, &HashSet::new(), 6).await;
        server.await.unwrap();

        assert_eq!(loaded.len(), 3);
        let mut contents: Vec<&str> = loaded.iter().map(|r| r.content.as_str()).collect();
        contents.sort();
        assert_eq!(contents, vec!["PNGDATA", "alert(1)", "body{}"]);
    }

    #[tokio::test]
    async fn fetch_resources_respects_block_types() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body = "body{}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let urls = vec![ResourceUrl {
            resource_type: ResourceType::Stylesheet,
            url: format!("{base}/style.css"),
        }];

        let mut block = HashSet::new();
        block.insert(ResourceType::Stylesheet);
        let loaded = fetch_resources(urls, &block, 6).await;

        assert!(loaded.is_empty());
        // server never got a connection since it was blocked
    }

    #[tokio::test]
    async fn fetch_resources_empty_input() {
        let loaded = fetch_resources(vec![], &HashSet::new(), 6).await;
        assert!(loaded.is_empty());
    }
}

#[cfg(test)]
#[cfg(feature = "proptest")]
mod proptests {
    use proptest::prelude::*;

    use super::*;
    use crate::html_parser::parse_html;

    fn url_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-zA-Z0-9/._-]", 1..20).prop_map(|chars| chars.join(""))
    }

    fn resource_tag_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            // link stylesheet
            url_strategy().prop_map(|href| format!(r#"<link rel="stylesheet" href="{}">"#, href)),
            // script src
            url_strategy().prop_map(|src| format!(r#"<script src="{}"></script>"#, src)),
            // img src
            url_strategy().prop_map(|src| format!(r#"<img src="{}">"#, src)),
            // video src
            url_strategy().prop_map(|src| format!(r#"<video src="{}"></video>"#, src)),
            // audio src
            url_strategy().prop_map(|src| format!(r#"<audio src="{}"></audio>"#, src)),
        ]
    }

    fn html_with_resources_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(resource_tag_strategy(), 0..10).prop_map(|tags| {
            format!(
                r#"<html><head>{}</head><body>{}</body></html>"#,
                tags[..tags.len().min(tags.len() / 2)].join(""),
                tags[tags.len().min(tags.len() / 2)..].join("")
            )
        })
    }

    proptest! {
        #[test]
        fn extracted_urls_are_nonempty_strings(html in html_with_resources_strategy()) {
            let dom = parse_html(&html);
            let urls = extract_resource_urls(&dom);
            for r in &urls {
                prop_assert!(!r.url.is_empty());
            }
        }

        #[test]
        fn block_filtering_removes_blocked_types(html in html_with_resources_strategy()) {
            let dom = parse_html(&html);
            let urls = extract_resource_urls(&dom);
            let mut block = HashSet::new();
            block.insert(ResourceType::Stylesheet);
            let filtered = filter_by_block_types(urls, &block);
            for r in &filtered {
                prop_assert_ne!(r.resource_type, ResourceType::Stylesheet);
            }
        }

        #[test]
        fn block_filtering_is_idempotent(html in html_with_resources_strategy()) {
            let dom = parse_html(&html);
            let urls = extract_resource_urls(&dom);
            let mut block = HashSet::new();
            block.insert(ResourceType::Script);
            let once = filter_by_block_types(urls.clone(), &block);
            let twice = filter_by_block_types(once.clone(), &block);
            prop_assert_eq!(once, twice);
        }
    }
}
