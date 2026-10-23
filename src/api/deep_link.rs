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
///
/// `legado://` 是注册到系统的协议，**任何网页**都能构造一条链接唤起本应用；导入目标
/// 又是任意 http(s) 地址，所以必须带上限 —— 具体实现见 `api::remote_text`，那里
/// 与朗读引擎导入共用同一份"边读边计数"的逻辑。
pub async fn read_remote_text_file_core(
    state: &crate::api::AppState,
    req: RemoteTextParam,
) -> Result<crate::error::error::ApiResponse<String>, crate::error::error::AppError> {
    use crate::api::remote_text::{fetch_capped_text, MAX_REMOTE_TEXT_BYTES};
    use crate::error::error::ApiResponse;

    let text = fetch_capped_text(
        &state.book_service.http_client(),
        &req.url,
        MAX_REMOTE_TEXT_BYTES,
    )
    .await?;
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
/// Windows 注册表把协议参数以 `"$exe" "%1"` 传入，所以 URL 通常是独立参数；也有
/// 少数启动器把整条命令行拼成一个参数（`"exe" "legado://..."`），因此先按引号与
/// 空白切成 token 再逐个判断，而不是在整串里 `find("legado://")`。
///
/// **为什么要切 token**：早先的实现对任意参数做子串匹配，于是
/// `--log-path=legado://x` 这种普通开关也会被当成深链，把用户带到一个莫名其妙的
/// 导入界面。现在只认两种形态：
/// 1. token 本身以 `legado://` 开头（去掉引号后）；
/// 2. `-` 开头的开关，`=` 右边是一个完整的 `legado://…`，如 `--url=legado://…`。
pub fn find_deeplink(args: &[String]) -> Option<String> {
    args.iter()
        .flat_map(|arg| split_tokens(arg))
        .find_map(deeplink_from_token)
}

/// 一个 token 是否是深链。
fn deeplink_from_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if token.to_ascii_lowercase().starts_with("legado://") {
        return Some(token.to_string());
    }
    // 显式开关形态。这里**按名单放行**而不是"任何以 - 开头的参数"：否则
    // `--log-path=legado://x` 这种普通开关也会唤起导入界面。
    for flag in ["--url=", "-url="] {
        if let Some(value) = strip_prefix_ignore_case(token, flag) {
            let value = value.trim().trim_matches('"');
            if value.to_ascii_lowercase().starts_with("legado://") {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 大小写不敏感地去掉前缀（前缀都是 ASCII，`get` 不会切在 UTF-8 边界中间）。
fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

/// 把可能含引号的命令行参数切成 token。引号内的空白保留（路径里有空格）。
fn split_tokens(arg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = arg;
    while !rest.is_empty() {
        let Some(open) = rest.find('"') else {
            out.extend(rest.split_whitespace());
            break;
        };
        let (before, tail) = rest.split_at(open);
        out.extend(before.split_whitespace());
        let tail = &tail[1..];
        match tail.find('"') {
            Some(close) => {
                out.push(&tail[..close]);
                rest = &tail[close + 1..];
            }
            // 引号没闭合：退化成按空白切，不要吞掉后面的参数。
            None => {
                out.extend(tail.split_whitespace());
                break;
            }
        }
    }
    out
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

    /// 普通开关里出现 `legado://` 子串不算深链。早先的实现会在参数里 `find()`，
    /// 于是 `--log-path=legado://x` 也能唤起导入界面。
    ///
    /// 注意这里只放"子串在开关内部"的形态：像 `--note=see legado://x` 这种带空格的
    /// 串会被切成两个 token，第二个本身就是合法深链 —— 那是命令行分词的固有歧义，
    /// 不是本函数能消除的（真实启动参数也已经是分开的两项）。
    #[test]
    fn ignores_deeplink_substring_in_unrelated_flags() {
        for flag in [
            "--log-path=legado://x",
            "--note=legado://import/x",
            "backup-legado://1.zip",
            "--url=legado:/broken",
        ] {
            let args = vec!["reader.exe".to_string(), flag.to_string()];
            assert_eq!(find_deeplink(&args), None, "should ignore {flag}");
        }
    }

    /// 整个命令行被拼成一个参数时（含空格与引号），也要能切出 URL。
    #[test]
    fn splits_a_single_concatenated_command_line() {
        let args = vec![
            "\"C:\\Program Files\\Reader-L\\reader-desktop.exe\" \"legado://import/bookSource?src=https://example.test/a.json\""
                .to_string(),
        ];
        assert_eq!(
            find_deeplink(&args).as_deref(),
            Some("legado://import/bookSource?src=https://example.test/a.json")
        );
    }

    /// `=` 右边带引号（启动器各自的写法不同）不该把引号带进 URL。
    #[test]
    fn strips_quotes_around_flag_value() {
        let args = vec![
            "reader.exe".to_string(),
            "--url=\"legado://import/theme?src=x\"".to_string(),
        ];
        assert_eq!(
            find_deeplink(&args).as_deref(),
            Some("legado://import/theme?src=x")
        );
    }

    /// 引号没闭合时退化成按空白切，不能把后面的参数一起吞掉。
    #[test]
    fn tolerates_unclosed_quotes() {
        let args = vec!["\"C:\\Program Files\\Reader-L\\reader-desktop.exe".to_string()];
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
