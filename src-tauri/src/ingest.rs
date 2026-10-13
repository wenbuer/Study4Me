//! 输入摄取：网页 / PDF / Word / Markdown / 纯文本 → 标题 + 正文。

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::llm::{truncate, LlmClient};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ingested {
    pub title: String,
    pub kind: String,
    pub source: String,
    pub text: String,
    pub chars: i64,
    /// 这份材料是怎么来的：模型整理 / 脚本解析 / 视觉转写。
    pub via: String,
}

impl Ingested {
    /// 直接粘贴的文本。
    pub fn from_text(title: String, text: String) -> Result<Self> {
        if text.trim().chars().count() < 40 {
            return Err(anyhow!("文本太短，至少 40 个字才够拆知识点"));
        }
        Ok(Ingested::new(title, "text", "粘贴文本".to_string(), text))
    }

    fn new(title: String, kind: &str, source: String, text: String) -> Self {
        let text = normalize(&text);
        Self {
            chars: text.chars().count() as i64,
            title: if title.trim().is_empty() { "未命名材料".into() } else { title.trim().to_string() },
            kind: kind.to_string(),
            source,
            text,
            via: "脚本解析".into(),
        }
    }

    pub fn with_via(mut self, via: &str) -> Self {
        self.via = via.to_string();
        self
    }
}

pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank_run = 0;
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// 抓取网页正文。
pub async fn from_url(http: &reqwest::Client, url: &str) -> Result<Ingested> {
    let resp = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) 致知/0.1")
        .send()
        .await
        .with_context(|| format!("请求失败：{url}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("网页返回 {}", status.as_u16()));
    }
    let html = resp.text().await?;
    let (title, text) = extract_article(&html)?;
    Ok(Ingested::new(title, "url", url.to_string(), text))
}

/// 从 HTML 抽标题与正文（结构化 Markdown，去掉导航与脚本噪音）。
pub fn extract_article(html: &str) -> Result<(String, String)> {
    let doc = Html::parse_document(html);
    let title = page_title(&doc);
    let mut body = article_markdown(&doc);

    // 结构化抽取几乎没拿到东西、但页面本身很大：多半是 JS 渲染或 div 堆版式，退回整页可见文本。
    if body.chars().count() < 500 && html.len() > 20_000 {
        let fallback = visible_text(&doc);
        if fallback.chars().count() > body.chars().count() {
            body = fallback;
        }
    }

    if body.trim().is_empty() {
        return Err(anyhow!("没能从页面里抽到正文，可以直接把正文粘贴进来"));
    }
    Ok((title, body))
}

fn page_title(doc: &Html) -> String {
    if let Ok(sel) = Selector::parse("meta[property='og:title']") {
        if let Some(node) = doc.select(&sel).next() {
            if let Some(content) = node.value().attr("content") {
                let cleaned = content.split_whitespace().collect::<Vec<_>>().join(" ");
                if !cleaned.trim().is_empty() {
                    return cleaned.trim().to_string();
                }
            }
        }
    }
    // 站点名常以第一个 h1 出现，取文字最长的 h1 更接近文章标题
    if let Ok(sel) = Selector::parse("h1") {
        let longest = doc
            .select(&sel)
            .map(|node| inline_text(&node))
            .max_by_key(|text| text.chars().count())
            .unwrap_or_default();
        if longest.chars().count() >= 4 {
            return longest;
        }
    }
    if let Ok(sel) = Selector::parse("title") {
        if let Some(node) = doc.select(&sel).next() {
            let text = node.text().collect::<String>();
            let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let head = cleaned.split(" - ").next().unwrap_or(&cleaned).trim();
            if !head.is_empty() {
                return head.to_string();
            }
        }
    }
    String::new()
}

/// 页面主内容 → Markdown：标题分级、列表、表格、代码块、引用都保留，交给模型的信息尽量不丢。
fn article_markdown(doc: &Html) -> String {
    let Ok(selector) = Selector::parse(
        "h1, h2, h3, h4, h5, h6, p, li, pre, blockquote, tr, dd, figcaption, td",
    ) else {
        return String::new();
    };
    let cell_selector = Selector::parse("td, th").ok();

    let mut out = String::new();
    for node in doc.select(&selector) {
        if is_noise(&node) || is_nested_text(&node) {
            continue;
        }
        match node.value().name() {
            name @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                let level = name[1..].parse::<usize>().unwrap_or(2).clamp(1, 6);
                let title = inline_text(&node);
                if !title.is_empty() {
                    out.push_str(&format!("{} {}\n\n", "#".repeat(level), title));
                }
            }
            "li" => {
                let item = inline_text(&node);
                if !item.is_empty() {
                    out.push_str(&format!("- {item}\n"));
                }
            }
            "pre" => {
                let code = node.text().collect::<String>();
                let code = code.trim_matches('\n');
                if !code.trim().is_empty() {
                    out.push_str(&format!("\n```\n{code}\n```\n\n"));
                }
            }
            "blockquote" => {
                let quote = inline_text(&node);
                if !quote.is_empty() {
                    out.push_str(&format!("> {quote}\n\n"));
                }
            }
            "tr" => {
                if let Some(sel) = &cell_selector {
                    let cells: Vec<String> = node.select(sel).map(|cell| inline_text(&cell)).collect();
                    if cells.iter().any(|cell| !cell.is_empty()) {
                        out.push_str(&format!("| {} |\n", cells.join(" | ")));
                    }
                }
            }
            // 单元格已经在 tr 里处理过，跳过以免重复
            "td" => continue,
            _ => {
                let paragraph = inline_text(&node);
                if paragraph.chars().count() >= 12 {
                    out.push_str(&paragraph);
                    out.push_str("\n\n");
                }
            }
        }
    }
    normalize(&out)
}

/// 导航、页脚、表单这类结构性噪音。
fn is_noise(node: &scraper::ElementRef) -> bool {
    node.ancestors().any(|ancestor| {
        ancestor
            .value()
            .as_element()
            .map(|element| {
                matches!(
                    element.name(),
                    "nav" | "header" | "footer" | "aside" | "script" | "style" | "noscript"
                        | "template" | "form" | "select" | "button"
                )
            })
            .unwrap_or(false)
    })
}

/// 已经落在别的文本块内部，避免同一段文字被重复收集。
fn is_nested_text(node: &scraper::ElementRef) -> bool {
    node.ancestors().skip(1).any(|ancestor| {
        ancestor
            .value()
            .as_element()
            .map(|element| {
                matches!(
                    element.name(),
                    "p" | "li" | "pre" | "blockquote" | "td" | "th" | "h1" | "h2" | "h3" | "h4"
                        | "h5" | "h6" | "dd" | "figcaption"
                )
            })
            .unwrap_or(false)
    })
}

fn inline_text(node: &scraper::ElementRef) -> String {
    node.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ── 学习单元切分：保留原文，不经过模型 ───────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unit {
    /// 所属章节（文档自带的「分类」，或材料目录）
    pub section: String,
    pub title: String,
    /// 完整讲解正文（Markdown 原文）
    pub content: String,
}

/// 把材料正文切成学习单元：一个文档 / 一个小节一个单元。
/// 存的是完整原文，不是摘要——学习页要让人真的读到东西。
pub fn split_units(body: &str, kind: &str) -> Vec<Unit> {
    if kind == "repo" {
        split_repo_units(body)
    } else {
        split_heading_units(body)
    }
}

fn split_repo_units(body: &str) -> Vec<Unit> {
    // 先按来源标记拆成一篇篇文档
    let mut docs: Vec<(String, String)> = Vec::new();
    for raw in body.split("<!-- 来源：") {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Some((path, rest)) = raw.split_once("-->") else {
            continue;
        };
        let content = rest.trim();
        if content.chars().count() < 80 {
            continue;
        }
        docs.push((path.trim().to_string(), content.to_string()));
    }

    // 目录 → 章节名：同一目录下只要有一篇写了「分类」，整组都用它，
    // 免得 RAG 和 rag 被拆成两个章节。
    let mut dir_names: HashMap<String, String> = HashMap::new();
    for (path, content) in &docs {
        let dir = path.split('/').next().unwrap_or("").to_string();
        if let Some(name) = meta_value(content, "分类") {
            dir_names.entry(dir).or_insert(name);
        }
    }

    let mut units = Vec::new();
    for (path, content) in &docs {
        // 仓库自身的说明与建设文档不是学习内容
        if path.ends_with("README.md")
            || path.starts_with("docs/")
            || path.starts_with(".github/")
            || path.starts_with("assets/")
        {
            continue;
        }
        let dir = path.split('/').next().unwrap_or("");
        let title = content
            .lines()
            .find(|line| line.trim_start().starts_with("# "))
            .map(|line| line.trim_start_matches("# ").trim().to_string())
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
        let section = dir_names
            .get(dir)
            .cloned()
            .or_else(|| meta_value(content, "分类"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| readable_dir(dir));
        units.push(Unit {
            section,
            title,
            content: content.clone(),
        });
    }
    units
}

/// 没有 Markdown 标题的正文（直接粘贴的文本、纯文本文件）用开头第一句像样的话当标题：
/// 跳过括号说明、标题行、引用行和过短的片段，最多取 32 个字。
fn first_meaningful_line(body: &str) -> Option<String> {
    body.lines()
        .map(|line| line.trim())
        .filter(|line| line.chars().count() >= 6)
        .find(|line| {
            !line.starts_with('（')
                && !line.starts_with('(')
                && !line.starts_with('#')
                && !line.starts_with('>')
        })
        .map(|line| line.chars().take(32).collect())
}

/// 网页 / 文档：按二级标题切小节；没有小节就整篇作为一个单元。
fn split_heading_units(body: &str) -> Vec<Unit> {
    let page_title = body
        .lines()
        .find(|line| line.trim_start().starts_with("# "))
        .map(|line| line.trim_start_matches("# ").trim().to_string())
        .or_else(|| first_meaningful_line(body))
        .unwrap_or_else(|| "材料".to_string());

    let mut units = Vec::new();
    let mut title: Option<String> = None;
    let mut buffer = String::new();

    for line in body.lines() {
        let is_section = line.starts_with("## ") && !line.starts_with("### ");
        if is_section {
            if let Some(previous) = title.take() {
                if buffer.trim().chars().count() > 80 {
                    units.push(Unit {
                        section: page_title.clone(),
                        title: previous,
                        content: buffer.trim().to_string(),
                    });
                }
            }
            title = Some(line.trim_start_matches("## ").trim().to_string());
            buffer.clear();
            buffer.push_str(line);
            buffer.push('\n');
        } else {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    if let Some(previous) = title {
        if buffer.trim().chars().count() > 80 {
            units.push(Unit {
                section: page_title.clone(),
                title: previous,
                content: buffer.trim().to_string(),
            });
        }
    }
    if units.is_empty() && !body.trim().is_empty() {
        units.push(Unit {
            section: page_title.clone(),
            title: page_title,
            content: body.trim().to_string(),
        });
    }
    units
}

/// 文档头部的「> 分类：xxx」这类元信息。
fn meta_value(body: &str, key: &str) -> Option<String> {
    body.lines()
        .take(20)
        .find(|line| line.trim_start().starts_with('>') && line.contains(key))
        .and_then(|line| line.split(['：', ':']).nth(1))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 按字符数截断（给模型的小段落用）。
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\n…（后文因长度截断）")
}

/// 01-agent-architecture → Agent Architecture
fn readable_dir(dir: &str) -> String {
    let trimmed = dir.split_once('-').map(|(_, rest)| rest).unwrap_or(dir);
    trimmed
        .split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 按字符数切块，尽量落在段落边界上。
pub fn split_by_chars(text: &str, limit: usize) -> Vec<String> {
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for line in text.split_inclusive('\n') {
        let line_chars = line.chars().count();
        if count + line_chars > limit && !current.trim().is_empty() {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
        current.push_str(line);
        count += line_chars;
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

/// 整页可见文本：逐元素取直接文本节点，跳过 script / style / noscript。
fn visible_text(doc: &Html) -> String {
    let selector = Selector::parse("body *").unwrap();
    let mut lines: Vec<String> = Vec::new();
    for node in doc.select(&selector) {
        let name = node.value().name();
        if matches!(name, "script" | "style" | "noscript" | "template") {
            continue;
        }
        let mut line = String::new();
        for child in node.children() {
            if let Some(text) = child.value().as_text() {
                line.push_str(text);
            }
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    lines.join("\n")
}

// ── GitHub 仓库：整仓文档批量导入 ─────────────────────────

/// 识别 GitHub 仓库地址；单个文件页交给普通抓取。
fn github_repo(url: &str) -> Option<String> {
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .strip_prefix("github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?.split(['?', '#']).next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    if matches!(
        parts.next().unwrap_or(""),
        "blob" | "raw" | "issues" | "pull" | "releases" | "actions" | "wiki" | "commit"
    ) {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// 把一个 GitHub 仓库里的 Markdown 文档全部抓下来，合成一份材料。
///
/// 仓库首页本身只有一份 README，直接交给模型只能拆出几个知识点；
/// 真正要学的内容在目录里的每一篇文档上，所以这里先把它们都取回来。
pub async fn from_github_repo(
    http: &reqwest::Client,
    repo: &str,
    source_url: &str,
    limit: usize,
) -> Result<Ingested> {
    let api = format!("https://api.github.com/repos/{repo}/git/trees/HEAD?recursive=1");
    let resp = http
        .get(&api)
        .header("User-Agent", UA)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("读取仓库目录失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub 接口返回 {}", resp.status().as_u16()));
    }
    let tree: serde_json::Value = resp.json().await.context("仓库目录格式异常")?;

    let mut paths: Vec<String> = tree["tree"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter(|node| node["type"] == "blob")
                .filter_map(|node| node["path"].as_str())
                .filter(|path| path.ends_with(".md"))
                .map(|path| path.to_string())
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    if paths.is_empty() {
        return Err(anyhow!("仓库里没有 Markdown 文档"));
    }
    let found = paths.len();
    paths.truncate(limit);

    // 并发抓原始 Markdown（raw 是纯文本，省掉解析 HTML）
    let docs: Vec<(String, String)> = futures::stream::iter(paths.into_iter())
        .map(|path| {
            let http = http.clone();
            let repo = repo.to_string();
            async move {
                let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/{path}");
                let resp = http
                    .get(&url)
                    .header("User-Agent", UA)
                    .send()
                    .await
                    .ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let text = resp.text().await.ok()?;
                if text.trim().is_empty() {
                    return None;
                }
                Some((path, text))
            }
        })
        .buffer_unordered(6)
        .filter_map(|item| async move { item })
        .collect()
        .await;

    if docs.is_empty() {
        return Err(anyhow!("仓库文档都没抓到，可能是网络问题"));
    }

    let mut body = String::new();
    for (path, text) in &docs {
        body.push_str(&format!("\n\n<!-- 来源：{} -->\n\n{}", path, text.trim()));
    }

    let title = format!("{repo} · {found} 篇文档");
    Ok(Ingested::new(title, "repo", source_url.to_string(), body).with_via("批量抓取"))
}

// ── 智能导入：模型优先，脚本兜底 ─────────────────────────

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0 Safari/537.36";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 抓页面：跟随 HTTP 重定向与 meta refresh，返回（最终 URL, 标题, 可见正文）。
pub async fn fetch_page(http: &reqwest::Client, url: &str) -> Result<(String, String, String)> {
    let mut current = url.to_string();
    let mut last_err = anyhow!("没能拿到页面内容");
    for _ in 0..3 {
        let resp = match http
            .get(&current)
            .header("User-Agent", UA)
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                last_err = anyhow!("请求失败：{e}");
                break;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            last_err = anyhow!("网页返回 {}", status.as_u16());
            break;
        }
        let final_url = resp.url().to_string();
        let html = match resp.text().await {
            Ok(html) => html,
            Err(e) => {
                last_err = anyhow!("读取网页失败：{e}");
                break;
            }
        };
        if let Some(next) = meta_refresh_target(&html) {
            if next != current {
                current = next;
                continue;
            }
        }
        return extract_article(&html).map(|(title, text)| (final_url, title, text));
    }
    Err(last_err)
}

/// 页面里的 meta refresh 跳转目标（很多分享链接靠这个跳转）。
fn meta_refresh_target(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("meta").ok()?;
    for node in doc.select(&selector) {
        let is_refresh = node
            .value()
            .attr("http-equiv")
            .map(|v| v.eq_ignore_ascii_case("refresh"))
            .unwrap_or(false);
        if !is_refresh {
            continue;
        }
        let Some(content) = node.value().attr("content") else { continue };
        let lower = content.to_ascii_lowercase();
        if let Some(idx) = lower.find("url=") {
            let target = content[idx + 4..]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            if target.starts_with("http") {
                return Some(target);
            }
        }
    }
    None
}

/// 用搜索引擎找资料（Bing 中国站，无需额外 Key）。
pub async fn search(http: &reqwest::Client, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let url = format!(
        "https://cn.bing.com/search?q={}&setlang=zh-CN",
        url_encode(query)
    );
    let html = http
        .get(&url)
        .header("User-Agent", UA)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
        .context("搜索请求失败")?
        .text()
        .await?;

    let doc = Html::parse_document(&html);
    let item_sel = Selector::parse("li.b_algo").unwrap();
    let link_sel = Selector::parse("h2 a").unwrap();
    let text_sel = Selector::parse("p").unwrap();

    let mut hits = Vec::new();
    for item in doc.select(&item_sel) {
        if hits.len() >= limit {
            break;
        }
        let Some(link) = item.select(&link_sel).next() else { continue };
        let href = link.value().attr("href").unwrap_or("").to_string();
        if !href.starts_with("http") {
            continue;
        }
        let title = link.text().collect::<String>().trim().to_string();
        let snippet = item
            .select(&text_sel)
            .next()
            .map(|n| n.text().collect::<String>())
            .unwrap_or_default();
        hits.push(SearchHit {
            title,
            url: href,
            snippet: snippet.trim().to_string(),
        });
    }
    if hits.is_empty() {
        return Err(anyhow!("搜索没有返回可用结果，换个说法，或者直接把文章链接给我"));
    }
    Ok(hits)
}

fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 从链接里猜搜索关键词（原链接打不开时用）。
fn url_keywords(url: &str) -> String {
    let tail = url.split("//").nth(1).unwrap_or(url);
    let path = tail.split(['?', '#']).next().unwrap_or(tail);
    let words: Vec<String> = path
        .split('/')
        .skip(1)
        .flat_map(|seg| seg.split(['-', '_', '.']))
        .filter(|w| w.chars().count() > 2 && !w.chars().all(|c| c.is_ascii_digit()))
        .map(|w| w.to_string())
        .collect();
    words.join(" ")
}

/// 交给模型整理；模型不可用或结果太短时返回 None，由调用方走脚本兜底。
pub async fn polish(
    llm: &LlmClient,
    origin: &str,
    fallback_title: &str,
    raw: &str,
) -> Option<(String, String)> {
    if !llm.ready() || raw.chars().count() < 120 {
        return None;
    }
    // 整仓导入这类超长材料不整理：几十万字要几十次调用，又贵又慢，
    // 而抓来的 Markdown 本身已经是完整结构，保留原文更划算。
    if raw.chars().count() > 40_000 {
        eprintln!("[整理] 材料 {} 字，超过整理阈值，保留原文", raw.chars().count());
        return None;
    }
    // 分块交给模型：一块约 4000 字，正好是模型一次能完整输出的量级，
    // 块太大模型会写到一半被截断，后半部分就丢了。
    let chunks = split_by_chars(raw, 4_000);
    let total = chunks.len();
    let mut parts: Vec<String> = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        let hint = if total == 1 {
            "整份材料".to_string()
        } else {
            format!("第 {}/{} 段", index + 1, total)
        };
        match llm.extract_material(origin, &hint, chunk).await {
            Ok(part) if part.chars().count() > 40 => parts.push(part),
            Ok(_) => eprintln!("[整理] {hint} 产出过短，跳过"),
            Err(e) => eprintln!("[整理] {hint} 失败：{e}"),
        }
    }

    if parts.is_empty() {
        eprintln!("[整理] 模型没有产出可用内容，改用脚本解析");
        return None;
    }
    Some((fallback_title.to_string(), parts.join("\n\n")))
}

/// 本地文件：脚本先抽文本，再让模型把断行、页眉页脚整理掉。
pub async fn polish_ingested(llm: &LlmClient, item: Ingested) -> Ingested {
    let origin = format!("本地文件 {}", item.source);
    if let Some((title, body)) = polish(llm, &origin, &item.title, &item.text).await {
        return Ingested::new(title, &item.kind, item.source.clone(), body).with_via("模型整理");
    }
    item
}

/// 智能导入一个链接：仓库整仓导入 / 单页抓取 → 模型整理；抓不到就改用搜索。
pub async fn smart_from_url(http: &reqwest::Client, llm: &LlmClient, url: &str) -> Result<Ingested> {
    // GitHub 仓库：README 只是目录，真正的学习内容在每篇文档里
    if let Some(repo) = github_repo(url) {
        match from_github_repo(http, &repo, url, 300).await {
            Ok(item) if item.chars > 2_000 => return Ok(item),
            Ok(_) => eprintln!("[批量] 仓库文档过少，回退单页抓取"),
            Err(e) => eprintln!("[批量] 整仓抓取失败，回退单页：{e}"),
        }
    }

    match fetch_page(http, url).await {
        Ok((final_url, title, text)) => {
            let origin = format!("网页 {final_url}");
            if let Some((new_title, body)) = polish(llm, &origin, &title, &text).await {
                return Ok(Ingested::new(new_title, "url", final_url, body).with_via("模型整理"));
            }
            Ok(Ingested::new(title, "url", final_url, text).with_via("脚本解析"))
        }
        Err(err) => {
            let query = url_keywords(url);
            if query.trim().is_empty() {
                return Err(anyhow!("{err}（也猜不出可搜索的关键词，试试直接粘贴正文）"));
            }
            let mut found = smart_from_topic(http, llm, &query).await?;
            found.source = format!("原链接打不开（{err}），改为搜索「{query}」");
            Ok(found)
        }
    }
}

/// 智能导入一个主题：搜索 → 抓前几条 → 模型整理成一份材料。
pub async fn smart_from_topic(http: &reqwest::Client, llm: &LlmClient, query: &str) -> Result<Ingested> {
    let hits = search(http, query, 4).await?;
    let mut chunks: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();

    for hit in hits.iter().take(3) {
        let page = fetch_page(http, &hit.url).await;
        match page {
            Ok((_, title, text)) if text.chars().count() > 200 => {
                chunks.push(format!(
                    "来源标题：{}\n来源链接：{}\n搜索摘要：{}\n正文：\n{}",
                    if title.trim().is_empty() { hit.title.clone() } else { title },
                    hit.url,
                    hit.snippet,
                    truncate(&text, 9000)
                ));
            }
            _ => {
                chunks.push(format!(
                    "来源标题：{}\n来源链接：{}\n搜索摘要：{}",
                    hit.title, hit.url, hit.snippet
                ));
            }
        }
        used.push(hit.url.clone());
    }

    let raw = chunks.join("\n\n---\n\n");
    let source_label = format!("搜索「{}」· {} 个来源", query, used.len());
    if let Some((title, body)) = polish(llm, &format!("网络搜索：{query}"), query, &raw).await {
        return Ok(Ingested::new(title, "topic", source_label, body).with_via("模型整理"));
    }
    Ok(Ingested::new(query.to_string(), "topic", source_label, raw).with_via("脚本解析"))
}

/// 按扩展名分派本地文件解析。
pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Ingested> {
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let title = name.rsplit(['/', '\\']).next().unwrap_or(name).to_string();
    match ext {
        "pdf" => {
            let text = pdf_extract::extract_text_from_mem(bytes).context("PDF 文本抽取失败（可能是扫描版，这种 PDF 里没有文字层）")?;
            Ok(Ingested::new(strip_ext(&title), "pdf", title, text))
        }
        "docx" => {
            let text = read_docx_text(bytes)?;
            Ok(Ingested::new(strip_ext(&title), "docx", title, text))
        }
        "md" | "markdown" | "txt" | "text" | "rs" | "py" | "js" | "ts" | "json" | "yaml" | "yml" | "csv" => {
            let text = String::from_utf8_lossy(bytes).to_string();
            Ok(Ingested::new(strip_ext(&title), "text", title, text))
        }
        _ => Err(anyhow!("暂不支持的文件类型：.{ext}（支持 pdf / docx / md / txt）")),
    }
}

fn strip_ext(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// docx 就是 zip + XML：直接抽 w:t 文本，不依赖第三方 API 细节。
fn read_docx_text(bytes: &[u8]) -> Result<String> {
use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("docx 不是有效的 zip 包")?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .context("docx 缺少 word/document.xml")?
        .read_to_string(&mut xml)?;
    let mut out = String::new();
    let mut in_text = false;
    let mut buf = String::new();
    let chars: Vec<char> = xml.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            let mut tag = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '>' {
                tag.push(chars[i]);
                i += 1;
            }
            i += 1;
            let tag_trim = tag.trim();
            if tag_trim.starts_with("w:t") && !tag_trim.ends_with('/') {
                in_text = true;
            } else if tag_trim == "/w:t" {
                in_text = false;
            } else if tag_trim.starts_with("/w:p") {
                out.push_str(buf.trim_end());
                out.push('\n');
                buf.clear();
            } else if tag_trim.starts_with("w:br") {
                buf.push('\n');
            }
        } else {
            if in_text {
                buf.push(chars[i]);
            }
            i += 1;
        }
    }
    if !buf.trim().is_empty() {
        out.push_str(buf.trim_end());
    }
    if out.trim().is_empty() {
        return Err(anyhow!("docx 里没有抽到文本"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_article_text_and_title() {
        let html = "<html><head><title>测试标题</title></head><body>\
            <nav>首页 产品 关于</nav>\
            <article><p>这是第一段正文内容，足够长以便通过长度过滤。</p>\
            <p>这是第二段正文内容，同样足够长。</p></article>\
            <script>var x = 1;</script></body></html>";
        let (title, text) = extract_article(html).unwrap();
        assert_eq!(title, "测试标题");
        assert!(text.contains("第一段正文"));
        assert!(!text.contains("var x"));
    }

    #[test]
    fn normalize_collapses_blank_lines() {
        assert_eq!(normalize("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn text_without_headings_gets_a_title_from_its_first_line() {
        let body = "（这段是粘贴时自带的说明，不该当标题）\n\n                    Claude Code 的 Session 机制\n\n正文从这里开始，讲的是 JSONL 记录。";
        let units = split_units(body, "text");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].title, "Claude Code 的 Session 机制");
    }

    #[test]
    fn keeps_the_fallback_title_when_there_is_nothing_to_use() {
        assert_eq!(first_meaningful_line("（只有一句说明）\n\n短"), None);
        let units = split_units("短", "text");
        assert_eq!(units[0].title, "材料");
    }
}