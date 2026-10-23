//! 远程 JSON 文本的下载与大小上限。
//!
//! 书源 / RSS / 净化规则 / 朗读引擎都支持「填一个直链，把整份集合导进来」。这类
//! 链接可以指向任意站点，长度完全由对方决定：没有上限时，一个几 GB 的响应就能把
//! 内存吃干，而且 `.text()` 是**先把整个响应读进内存再解码**，判长度已经来不及了。
//!
//! 所以统一走这里：先看 `Content-Length`，没有（分块响应）就逐块累计，超限立刻
//! 中止。上限按"最大的在线集合实测 < 2MB"留足余量取 8MB。

use crate::error::error::AppError;

/// 远程导入文件的大小上限。8MB 足够容纳最大的书源/规则集合。
pub const MAX_REMOTE_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// 只接受 http(s)：`legado://`、`file://` 之类的 URL 交给 reqwest 会得到难以理解
/// 的 "builder error"，提前挡下来并给出可读原因。
pub fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("http://") || url.starts_with("https://")
}

/// 拉取一份远程文本，超过 `max_bytes` 立即中止。
pub async fn fetch_capped_text(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<String, AppError> {
    let url = url.trim();
    if !is_http_url(url) {
        return Err(AppError::BadRequest(
            "导入链接必须以 http:// 或 https:// 开头".to_string(),
        ));
    }

    let response = client.get(url).send().await.map_err(|error| {
        tracing::error!("Failed to fetch import text from {url}: {error:?}");
        AppError::BadRequest(format!("网络请求失败: {error}"))
    })?;

    // 先看声明长度，能省掉一次无用的下载。
    if let Some(length) = response.content_length() {
        if length as usize > max_bytes {
            return Err(too_large(length as usize, max_bytes));
        }
    }

    // 没有 Content-Length 时（分块响应）只能边读边数：用 `chunk()` 逐块累计，
    // 超限立刻中止。不能先 `.bytes()` 再判长度 —— 那一步已经把整个响应读进内存了。
    let mut buffer: Vec<u8> = Vec::new();
    let mut response = response;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| AppError::BadRequest(format!("读取响应失败: {error}")))?;
        let Some(chunk) = chunk else { break };
        if buffer.len() + chunk.len() > max_bytes {
            return Err(too_large(buffer.len() + chunk.len(), max_bytes));
        }
        buffer.extend_from_slice(&chunk);
    }

    let text = String::from_utf8_lossy(&buffer).to_string();
    if text.trim().is_empty() {
        return Err(AppError::BadRequest("远程文件内容为空".to_string()));
    }
    Ok(text)
}

/// 超限时的错误信息。有 Content-Length 时能报出实际大小，分块时只能报"至少多少"。
fn too_large(actual: usize, max_bytes: usize) -> AppError {
    AppError::BadRequest(format!(
        "导入文件过大（至少 {} MB），上限 {} MB",
        actual / 1024 / 1024,
        max_bytes / 1024 / 1024
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_http_schemes() {
        assert!(is_http_url("https://example.test/a.json"));
        assert!(is_http_url("  http://example.test/a.json  "));
        // 这些交给 reqwest 会得到 "builder error"，必须在下载前挡下。
        assert!(!is_http_url("legado://import/x"));
        assert!(!is_http_url("file:///C:/a.json"));
        assert!(!is_http_url("example.test/a.json"));
        assert!(!is_http_url(""));
    }

    #[test]
    fn too_large_message_reports_megabytes() {
        let message = too_large(9 * 1024 * 1024, MAX_REMOTE_TEXT_BYTES).to_string();
        assert!(message.contains('9'), "{message}");
        assert!(message.contains('8'), "{message}");
    }
}
