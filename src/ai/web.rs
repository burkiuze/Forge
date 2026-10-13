//! Web access straight from the user's device: search (DuckDuckGo HTML, Bing fallback) and page fetch.
//! No API key needed — requests go out from this machine like a browser would.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36 Forge";

fn get(url: &str) -> Result<String, String> {
    let agent = crate::ai::client::builder()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(20))
        .redirects(5)
        .build();
    match agent
        .get(url)
        .set("User-Agent", UA)
        .set("Accept-Language", "en-US,en;q=0.8,tr;q=0.7")
        .call()
    {
        Ok(r) => {
            let mut s = String::new();
            use std::io::Read;
            r.into_reader()
                .take(3_000_000)
                .read_to_string(&mut s)
                .map_err(|e| e.to_string())?;
            Ok(s)
        }
        Err(ureq::Error::Status(c, _)) => Err(format!("HTTP {c}")),
        Err(e) => Err(e.to_string()),
    }
}

pub fn search(query: &str, max: usize) -> Result<Vec<SearchHit>, String> {
    let q = encode(query);
    let mut hits = get(&format!("https://html.duckduckgo.com/html/?q={q}"))
        .map(|h| parse_ddg(&h))
        .unwrap_or_default();
    if hits.is_empty() {
        hits = parse_bing(&get(&format!(
            "https://www.bing.com/search?q={q}&setlang=en"
        ))?);
    }
    hits.truncate(max.max(1));
    if hits.is_empty() {
        return Err("no results (search engines unreachable?)".into());
    }
    Ok(hits)
}

/// Fetch a page and return readable text (title + body, scripts/styles removed).
pub fn fetch_text(url: &str, max_chars: usize) -> Result<String, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    let html = get(url)?;
    let title = between(&html, "<title", "</title>")
        .map(|t| t.split_once('>').map(|x| x.1).unwrap_or(t))
        .map(|t| decode_entities(t.trim()))
        .unwrap_or_default();
    let text = html_to_text(&html);
    let mut out = format!("# {title}\n{url}\n\n{text}");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>() + "…";
    }
    Ok(out)
}

pub fn parse_ddg(html: &str) -> Vec<SearchHit> {
    let mut out = vec![];
    for block in html.split("class=\"result__a\"").skip(1) {
        let Some(href) = attr(block, "href") else {
            continue;
        };
        let url = if let Some(i) = href.find("uddg=") {
            let enc = &href[i + 5..];
            decode(enc.split('&').next().unwrap_or(enc))
        } else if href.starts_with("//") {
            format!("https:{href}")
        } else {
            href.to_string()
        };
        let title = block
            .split_once('>')
            .map(|x| x.1)
            .and_then(|r| r.split("</a>").next())
            .map(strip_tags)
            .unwrap_or_default();
        let snippet = block
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|r| r.split_once('>'))
            .and_then(|(_, r)| r.split("</a>").next())
            .map(strip_tags)
            .unwrap_or_default();
        if url.starts_with("http") && !url.contains("duckduckgo.com/y.js") {
            out.push(SearchHit {
                title,
                url,
                snippet,
            });
        }
    }
    out
}

pub fn parse_bing(html: &str) -> Vec<SearchHit> {
    let mut out = vec![];
    for block in html.split("<li class=\"b_algo\"").skip(1) {
        let Some(h2) = between(block, "<h2", "</h2>") else {
            continue;
        };
        let Some(url) = attr(h2, "href") else {
            continue;
        };
        let title = h2
            .split_once("\">")
            .or_else(|| h2.split_once('>'))
            .map(|x| strip_tags(x.1))
            .unwrap_or_default();
        let snippet = between(block, "<p", "</p>")
            .map(|p| strip_tags(p.split_once('>').map(|x| x.1).unwrap_or(p)))
            .unwrap_or_default();
        if url.starts_with("http") {
            out.push(SearchHit {
                title,
                url: decode_entities(url),
                snippet,
            });
        }
    }
    out
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let i = s.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &s[i..];
    Some(&rest[..rest.find('"')?])
}

fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> {
    let i = s.find(a)? + a.len();
    let rest = &s[i..];
    Some(&rest[..rest.find(b)?])
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(out.split_whitespace().collect::<Vec<_>>().join(" ").trim())
}

pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in [
        "script", "style", "noscript", "svg", "head", "nav", "footer",
    ] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(a) = lower.find(&format!("<{tag}")) else {
                break;
            };
            let end_tag = format!("</{tag}>");
            match lower[a..].find(&end_tag) {
                Some(b) => s.replace_range(a..a + b + end_tag.len(), " "),
                None => {
                    s.truncate(a);
                    break;
                }
            }
        }
    }
    for br in [
        "</p>", "<br", "</li>", "</h1>", "</h2>", "</h3>", "</div>", "</tr>",
    ] {
        s = s.replace(br, &format!("\n{br}"));
    }
    let mut lines = vec![];
    for line in strip_tags_keep_lines(&s).lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.len() > 2 {
            lines.push(decode_entities(&l));
        }
    }
    lines.join("\n")
}

fn strip_tags_keep_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn decode_entities(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ");
    // Numeric entities.
    while let Some(i) = out.find("&#") {
        let Some(j) = out[i..].find(';') else { break };
        let num = &out[i + 2..i + j];
        let code = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(h, 16).ok()
        } else {
            num.parse().ok()
        };
        let ch = code
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default();
        out.replace_range(i..i + j + 1, &ch);
    }
    out
}

pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
