//! `legado://` 深链的接收端。
//!
//! 书源站点上的「一键导入」按钮是 `legado://import/<kind>?src=<url>` 形式的链接，
//! 只有注册了该协议的客户端才会被系统唤起。桌面版以前完全没接这一层，所以点了
//! 没有任何反应；现在安装包会注册协议，启动参数里的 URL 经这里暂存，等前端就绪
//! 后由 `take_pending_legado_deeplink` 取走。
//!
//! 为什么用"暂存 + 前端来取"而不是直接 emit 事件：冷启动时 URL 在进程启动参数里，
//! 那一刻 webview 还没建好，事件发出去无人接收。暂存到前端挂载后主动取，冷启动
//! 和"应用已在运行时被再次唤起"走同一条路径。
//!
//! 注意这里是**单槽**而非队列：用户连点两次只应导入最后一份，堆积历史链接没有意义。

use std::sync::Mutex;

static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// 记录一个待处理的深链。桌面壳在启动参数与单实例回调里调用。
pub fn set_pending(url: impl Into<String>) {
    let url = url.into();
    if url.trim().is_empty() {
        return;
    }
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(url);
    }
}

/// `take_pending_legado_deeplink` 的共享实现 (Tauri command 与 Web HTTP handler 共用)。
///
/// 返回 `ApiResponse` 信封与其它命令保持一致：前端统一用 `invokeEnvelope` 解包，
/// 若这里直接返回裸 `Option`，调用方就得为它单独走一条 `invoke` 路径。
pub fn take_pending_legado_deeplink_core() -> crate::error::error::ApiResponse<Option<String>> {
    let url = PENDING.lock().ok().and_then(|mut slot| slot.take());
    crate::error::error::ApiResponse::ok(url)
}

/// `take_pending_legado_deeplink` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub fn take_pending_legado_deeplink(
) -> crate::error::error::ApiResponse<Option<String>> {
    take_pending_legado_deeplink_core()
}

#[derive(Debug, serde::Deserialize)]
pub struct RemoteTextParam {
    pub url: String,
}

/// 拉取一份远程 JSON 文本的共享实现（导入净化规则这类"只在前端解析"的数据用）。
///
/// 为什么不直接让前端 `fetch`：导入链接可以指向任意站点，而 WebView 的跨域限制
/// 取决于对方是否给 CORS 头；走桌面端的 reqwest 就没有这个不确定性，也能复用用户
/// 配置的代理。书源/RSS 已有各自的专用命令，这里补的是通用兜底。
pub async fn read_remote_text_file_core(
    state: &crate::api::AppState,
    req: RemoteTextParam,
) -> Result<crate::error::error::ApiResponse<String>, crate::error::error::AppError> {
    use crate::error::error::{ApiResponse, AppError};

    let url = req.url.trim();
    // 只允许 http(s)：`legado://` 之类的 URL 交给 reqwest 会得到难以理解的
    // "builder error"，提前挡下来并给出可读原因。
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "导入链接必须以 http:// 或 https:// 开头".to_string(),
        ));
    }

    let client = state.book_service.http_client();
    let text = client
        .get(url)
        .send()
        .await
        .map_err(|error| {
            tracing::error!("Failed to fetch import text from {url}: {error:?}");
            AppError::BadRequest(format!("网络请求失败: {error}"))
        })?
        .text()
        .await
        .map_err(|error| AppError::BadRequest(format!("读取响应失败: {error}")))?;

    if text.trim().is_empty() {
        return Err(AppError::BadRequest("远程文件内容为空".to_string()));
    }
    Ok(ApiResponse::ok(text))
}

/// `read_remote_text_file` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn read_remote_text_file(
    state: tauri::State<'_, crate::api::AppState>,
    req: RemoteTextParam,
) -> Result<crate::error::error::ApiResponse<String>, crate::error::error::AppError> {
    read_remote_text_file_core(&state, req).await
}

/// 从命令行参数里挑出第一个 `legado://` 链接。
///
/// Windows 注册表把协议参数以 `"$exe" "%1"` 传入，所以 URL 可能是独立参数；浏览器
/// 也可能把整串拼成一个参数，因此这里既做精确前缀匹配，也在单参数内查找。
pub fn find_deeplink(args: &[String]) -> Option<String> {
    for arg in args {
        let trimmed = arg.trim().trim_matches('"');
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("legado://") {
            return Some(trimmed.to_string());
        }
        // 形如 `--url=legado://...` 之类：找出内嵌的 URL 起点。命令行开关本身不会
        // 含 `legado://`，所以这里不必先过滤以 `-` 开头的参数。
        if let Some(index) = lower.find("legado://") {
            return Some(trimmed[index..].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_argument() {
        let args = vec![
            "reader-desktop.exe".to_string(),
            "legado://import/bookSource?src=https://example.test/a.json".to_string(),
        ];
        assert_eq!(
            find_deeplink(&args).as_deref(),
            Some("legado://import/bookSource?src=https://example.test/a.json")
        );
    }

    #[test]
    fn finds_quoted_and_embedded_url() {
        let args = vec![
            "reader.exe".to_string(),
            "\"legado://import/rssSource?src=https://example.test/b.json\"".to_string(),
        ];
        assert!(find_deeplink(&args).unwrap().starts_with("legado://import/rssSource"));

        let embedded = vec!["reader.exe".to_string(), "--url=legado://import/theme?src=x".to_string()];
        assert_eq!(
            find_deeplink(&embedded).as_deref(),
            Some("legado://import/theme?src=x")
        );
    }

    #[test]
    fn ignores_flags_and_plain_paths() {
        let args = vec![
            "C:\\Program Files\\Reader-L\\reader-desktop.exe".to_string(),
            "--no-sandbox".to_string(),
            "C:\\Users\\x\\book.txt".to_string(),
        ];
        assert_eq!(find_deeplink(&args), None);
    }

    #[test]
    fn pending_slot_keeps_last_and_is_taken_once() {
        set_pending("legado://import/bookSource?src=1");
        set_pending("legado://import/bookSource?src=2");
        assert_eq!(
            take_pending_legado_deeplink().data.flatten().as_deref(),
            Some("legado://import/bookSource?src=2")
        );
        assert_eq!(take_pending_legado_deeplink().data.flatten(), None);
    }
}
