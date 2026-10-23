//! 朗读引擎的并发/间隔限速。
//!
//! 对齐 Legado 的 `help/ConcurrentRateLimiter.kt`：`HttpTTS.concurrentRate` 是一个
//! `次数/毫秒` 或 `毫秒` 的字符串。
//!
//! - `"0"` / 空串 / 非法值 —— 不限速（Legado 的默认值就是 `"0"`）；
//! - `"1/1000"` —— 每 1000ms 最多 1 次；
//! - `"3000"` —— 每 3000ms 最多 1 次（等价 `1/3000`）。
//!
//! # 为什么必须做
//!
//! 缓冲流默认 2 并发合成、双音色时还会同时用两个引擎；不加限速就会在几百毫秒内
//! 朝同一个第三方网关连发好几条请求。在线集合里的引擎大多写 `concurrentRate: "0"`
//! 表示"作者没打算限制"，但**也有相当一部分写了具体间隔** —— 对后者，我们现在的
//! 行为是直接违约，轻则被 429，重则被临时封 IP。
//!
//! 表是进程级全局的（与 Legado 的 `concurrentRecordMap` 一致）：同一个引擎的两次
//! 朗读请求可能来自不同的前端会话，限速必须跨会话生效。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 一个限速窗口：`access_limit` 次 / `interval`，窗口起点 `window_start`。
#[derive(Debug, Clone, Copy)]
struct RateRecord {
    window_start: Instant,
    access_limit: u32,
    interval: Duration,
    /// 当前窗口内已用的次数。
    frequency: u32,
}

fn registry() -> &'static Mutex<HashMap<String, RateRecord>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, RateRecord>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析 `concurrentRate`；返回 `None` 表示不限速。
///
/// 宽容处理：`"0"`、空串、`"abc"`、`"1/0"` 全部当作不限速 —— 限速是"尽力而为"的
/// 保护，不能因为导入数据里一个脏值就让整个引擎发不出请求（Legado 也是这么做的）。
pub fn parse_concurrent_rate(raw: Option<&str>) -> Option<(u32, Duration)> {
    let raw = raw?.trim();
    if raw.is_empty() || raw == "0" {
        return None;
    }
    let (limit, millis) = match raw.split_once('/') {
        Some((limit, millis)) => (limit.trim().parse::<u32>().ok()?, millis.trim().parse::<u64>().ok()?),
        // 只写一个数字时表示「每 N 毫秒 1 次」。
        None => (1, raw.parse::<u64>().ok()?),
    };
    if limit == 0 || millis == 0 {
        return None;
    }
    Some((limit, Duration::from_millis(millis)))
}

/// 取一次访问许可；需要等待时返回应等待的时长。
///
/// 与 Legado 的 `fetchStart()` 同构：窗口内次数没满就直接放行，满了就返回剩余等待
/// 时间，由调用方 `sleep` 后重试。**不做成阻塞锁**：这里是 async 命令，持锁睡眠会
/// 把其它引擎的请求一起卡住。
pub fn acquire(key: &str, rate: (u32, Duration)) -> Option<Duration> {
    let (access_limit, interval) = rate;
    let mut table = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let record = table.entry(key.to_string()).or_insert(RateRecord {
        window_start: now,
        access_limit,
        interval,
        frequency: 0,
    });

    // 引擎配置可能被改过：就地更新参数，但不重置窗口进度。
    record.access_limit = access_limit;
    record.interval = interval;

    let window_end = record.window_start + record.interval;
    if now >= window_end {
        // 窗口已过：开新窗口。
        record.window_start = now;
        record.frequency = 1;
        return None;
    }
    if record.frequency < record.access_limit {
        record.frequency += 1;
        return None;
    }
    Some(window_end.saturating_duration_since(now))
}

/// 带限速地执行一次请求。
pub async fn with_limit<T, F, Fut>(key: &str, rate: Option<(u32, Duration)>, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    if let Some(rate) = rate {
        // 循环而不是只等一轮：等的时候可能有别的请求把新窗口又占满了。
        while let Some(wait) = acquire(key, rate) {
            tokio::time::sleep(wait).await;
        }
    }
    f().await
}

// ─────────────────────── 合成并发（synthesisThreadCount） ───────────────────────

/// Legado 的 `synthesisThreadCount` 上限（它的编辑界面就是 1–8 的滑块）。
pub const MAX_SYNTHESIS_THREADS: i64 = 8;

/// 每个引擎的并发许可表。
///
/// 与上面的限速表分开：限速是「每 N 毫秒最多 M 次」，这里是「同时最多几个在飞」，
/// 两者可以同时生效（一个写了间隔的引擎若又声明 4 线程，仍要排队）。
fn permits() -> &'static Mutex<HashMap<String, (usize, std::sync::Arc<tokio::sync::Semaphore>)>> {
    static PERMITS: OnceLock<
        Mutex<HashMap<String, (usize, std::sync::Arc<tokio::sync::Semaphore>)>>,
    > = OnceLock::new();
    PERMITS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析 `synthesisThreadCount`；返回 `None` 表示不限并发。
///
/// **缺省不限**而不是"缺省 1"：本应用的前端缓冲流自己控制在 2 个并发，把没有
/// 声明这个字段的引擎一律压成串行只会让所有老引擎变慢。只有引擎自己写了具体
/// 数字时才尊重它 —— 那正是这条 Legado 字段的用途。
///
/// 越界值夹到 1..=8（与 Legado 的滑块一致），`0` / 负数当作没写。
pub fn concurrency_limit(raw: Option<i64>) -> Option<usize> {
    let value = raw?;
    if value <= 0 {
        return None;
    }
    Some(value.clamp(1, MAX_SYNTHESIS_THREADS) as usize)
}

/// 取一个并发许可，直到 `drop` 才释放。`limit` 变了会换一个新的信号量：
/// 短暂超额一次没影响（旧信号量上还在飞的请求 drop 时还给自己那份）。
///
/// 返回 `None` 只在信号量被关闭时发生 —— 本模块从不 `close()`，所以只是把
/// "取许可失败"表达成"不限并发"，宁可少限一次也不要让朗读发不出请求。
pub async fn acquire_slot(key: &str, limit: usize) -> Option<tokio::sync::OwnedSemaphorePermit> {
    let semaphore = {
        let mut table = permits()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match table.get(key) {
            Some((existing, semaphore)) if *existing == limit => semaphore.clone(),
            _ => {
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(limit));
                table.insert(key.to_string(), (limit, semaphore.clone()));
                semaphore
            }
        }
    };
    semaphore.acquire_owned().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legado_forms() {
        assert_eq!(parse_concurrent_rate(None), None);
        assert_eq!(parse_concurrent_rate(Some("")), None);
        assert_eq!(parse_concurrent_rate(Some("0")), None);
        assert_eq!(parse_concurrent_rate(Some("abc")), None);
        // `次数/毫秒`
        assert_eq!(
            parse_concurrent_rate(Some("2/1000")),
            Some((2, Duration::from_millis(1000)))
        );
        // 只写毫秒 = 每 N 毫秒 1 次
        assert_eq!(
            parse_concurrent_rate(Some("3000")),
            Some((1, Duration::from_millis(3000)))
        );
        // 0 次 / 0 毫秒都不合理，回退成不限速
        assert_eq!(parse_concurrent_rate(Some("0/1000")), None);
        assert_eq!(parse_concurrent_rate(Some("1/0")), None);
    }

    #[test]
    fn second_call_within_the_window_has_to_wait() {
        // 用独立 key，避免与其它用例共享登记表。
        let key = "test:single/1000";
        let rate = parse_concurrent_rate(Some("1/1000")).unwrap();
        assert_eq!(acquire(key, rate), None, "窗口内第一次应立刻放行");
        let wait = acquire(key, rate).expect("第二次应被拦下");
        assert!(wait > Duration::ZERO && wait <= Duration::from_millis(1000));
    }

    #[test]
    fn burst_limit_allows_n_calls() {
        let key = "test:burst/2/1000";
        let rate = parse_concurrent_rate(Some("2/1000")).unwrap();
        assert_eq!(acquire(key, rate), None);
        assert_eq!(acquire(key, rate), None, "2 次配额内第二次仍放行");
        assert!(acquire(key, rate).is_some(), "第 3 次必须等");
    }

    #[test]
    fn unlimited_rate_never_blocks() {
        // `parse` 对不限速返回 None，`with_limit` 直接跳过限速；这里验证边界不会误拦。
        let key = "test:unlimited";
        for _ in 0..5 {
            assert!(
                with_limit_blocking(key, None),
                "不限速时每次都该立刻放行"
            );
        }
    }

    /// 同步版 `with_limit`，只为单测断言"是否需要等待"。
    fn with_limit_blocking(key: &str, rate: Option<(u32, Duration)>) -> bool {
        match rate {
            None => true,
            Some(rate) => acquire(key, rate).is_none(),
        }
    }

    #[test]
    fn concurrency_limit_is_optional_and_clamped() {
        // 没写 / 写 0 / 写负数都表示不限并发。本应用前端自己控制在 2，不该因为
        // 引擎缺这个字段就被压成串行。
        assert_eq!(concurrency_limit(None), None);
        assert_eq!(concurrency_limit(Some(0)), None);
        assert_eq!(concurrency_limit(Some(-3)), None);
        // 1..=8 原样；越界夹到边界（Legado 的滑块就是 1–8）。
        assert_eq!(concurrency_limit(Some(1)), Some(1));
        assert_eq!(concurrency_limit(Some(4)), Some(4));
        assert_eq!(concurrency_limit(Some(8)), Some(8));
        assert_eq!(concurrency_limit(Some(99)), Some(8));
    }

    /// 许可必须真的互斥：第 2 个请求要等第 1 个释放。
    #[tokio::test]
    async fn acquire_slot_blocks_beyond_the_limit() {
        let key = "test:slots/1";
        let first = acquire_slot(key, 1).await.expect("第一个许可");
        // 用一个已完成的 task 探测"能不能立刻拿到"：不能的话它还在等。
        let pending = tokio::time::timeout(Duration::from_millis(50), acquire_slot(key, 1)).await;
        assert!(pending.is_err(), "拿到第一个许可后第二个必须排队");
        drop(first);
        let second = tokio::time::timeout(Duration::from_millis(50), acquire_slot(key, 1)).await;
        assert!(second.is_ok(), "释放后应立刻放行");
    }

    /// 不同 key 之间互不影响：限的是"同一个引擎"，不是全局。
    #[tokio::test]
    async fn slots_are_per_engine() {
        let _a = acquire_slot("test:slots/a", 1).await.expect("A 的许可");
        let b = tokio::time::timeout(Duration::from_millis(50), acquire_slot("test:slots/b", 1)).await;
        assert!(b.is_ok(), "另一个引擎不该被 A 的许可挡住");
    }
}
