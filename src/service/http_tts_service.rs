use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::error::AppError;
use crate::model::http_tts::HttpTts;
use crate::service::json_document_service::JsonDocumentService;

/// 存储用的文档名，沿用 Legado 的文件名，便于按 Legado 的数据目录理解。
const DOC_NAME: &str = "httpTts.json";

/// 一次批量保存的结果。前端用它提示「导入 N 条 / 跳过 M 条」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveSummary {
    pub saved: usize,
    pub skipped: usize,
}

/// Legado 朗读引擎的存储层。
///
/// 与书源/净化规则一样，数据是一份完整的 JSON 列表，落在 `json_documents` 表里。
/// 读改写操作全部串行化（同一把写锁）：导入常常是「先拉到列表再一次性 save」，
/// 并发保存若各自读到旧列表，后写的会覆盖先写的。
pub struct HttpTtsService {
    docs: Arc<JsonDocumentService>,
    write_lock: Arc<Mutex<()>>,
}

impl HttpTtsService {
    pub fn new(docs: Arc<JsonDocumentService>) -> Self {
        Self {
            docs,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// 列出全部引擎，按名字升序（同名按 id）。
    ///
    /// 前端下拉按名字查找引擎，而列表的 push 顺序取决于导入先后；不排序的话
    /// 同一份数据在两次启动间顺序可能变化，用户会以为引擎「自己换了位置」。
    pub async fn list(&self, user_ns: &str) -> Result<Vec<HttpTts>, AppError> {
        let mut list: Vec<HttpTts> = self.docs.read_list(user_ns, DOC_NAME).await?;
        list.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(list)
    }

    /// 按 id 或名字查一条。`key` 是前端传下来的 `ttsId`：数字形态按 id 命中，
    /// 名字形态（旧配置或手写调用）按名字命中。
    pub async fn find(&self, user_ns: &str, key: &str) -> Result<Option<HttpTts>, AppError> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let list: Vec<HttpTts> = self.docs.read_list(user_ns, DOC_NAME).await?;
        if let Ok(id) = key.parse::<i64>() {
            if let Some(found) = list.iter().find(|tts| tts.id == id) {
                return Ok(Some(found.clone()));
            }
        }
        Ok(list.into_iter().find(|tts| tts.name == key))
    }

    /// 批量 upsert（导入入口）。
    ///
    /// 匹配顺序：**先 name，再 id**。名字是用户在界面上唯一能认出的键，且 Legado
    /// 集合里的 id 是全网的导入时间戳 —— 再次导入同一份集合时 id 会撞，按 id 匹配
    /// 会把本地已有条目顶掉成另一端的数据。命中时保留本地 id，前端已经记住的
    /// `httpTtsId` 因此不会失效。
    ///
    /// 缺 `name` 或 `url` 的条目计入 `skipped`：没有地址的引擎发不出请求，存下来
    /// 只会让用户在下拉里选到一个必然失败的选项。
    pub async fn save_many(
        &self,
        user_ns: &str,
        items: Vec<HttpTts>,
    ) -> Result<SaveSummary, AppError> {
        let _guard = self.write_lock.lock().await;
        let mut list: Vec<HttpTts> = self.docs.read_list(user_ns, DOC_NAME).await?;
        let mut summary = SaveSummary::default();

        for mut item in items {
            item.name = item.name.trim().to_string();
            item.url = item.url.trim().to_string();
            if item.name.is_empty() || item.url.is_empty() {
                summary.skipped += 1;
                continue;
            }
            if let Some(pos) = list.iter().position(|tts| tts.name == item.name) {
                // 同名覆盖保留原 id：本地 id 是前端选中的引用，不能被导入数据换掉。
                item.id = list[pos].id;
                list[pos] = item;
                summary.saved += 1;
                continue;
            }
            if item.id != 0 {
                if let Some(pos) = list.iter().position(|tts| tts.id == item.id) {
                    list[pos] = item;
                    summary.saved += 1;
                    continue;
                }
            } else {
                item.id = next_id(&list);
            }
            list.push(item);
            summary.saved += 1;
        }

        // 全部被跳过时不必重写整份列表。
        if summary.saved > 0 {
            self.write_locked(user_ns, &list).await?;
        }
        Ok(summary)
    }

    /// 删除一条：`id` 非 0 且命中，或 `name` 命中，都算删到。返回实际删除条数 ——
    /// 前端提示要反映真实结果，不能一律回 true。
    pub async fn delete(
        &self,
        user_ns: &str,
        id: i64,
        name: &str,
    ) -> Result<usize, AppError> {
        let _guard = self.write_lock.lock().await;
        let mut list: Vec<HttpTts> = self.docs.read_list(user_ns, DOC_NAME).await?;
        let before = list.len();
        let name = name.trim();
        list.retain(|tts| !matches_target(tts, id, name));
        let deleted = before - list.len();
        if deleted > 0 {
            self.write_locked(user_ns, &list).await?;
        }
        Ok(deleted)
    }

    /// 批量删除。只读一次、只写一次：逐个 `delete` 会把整表重写 N 次，
    /// 中途失败还会留下删了一半的状态。
    pub async fn delete_many(
        &self,
        user_ns: &str,
        targets: &[HttpTts],
    ) -> Result<usize, AppError> {
        if targets.is_empty() {
            return Ok(0);
        }
        let _guard = self.write_lock.lock().await;
        let mut list: Vec<HttpTts> = self.docs.read_list(user_ns, DOC_NAME).await?;
        let before = list.len();
        for target in targets {
            let name = target.name.trim();
            list.retain(|tts| !matches_target(tts, target.id, name));
        }
        let deleted = before - list.len();
        if deleted > 0 {
            self.write_locked(user_ns, &list).await?;
        }
        Ok(deleted)
    }

    /// 落盘列表。**要求调用方已持有 write_lock**（见 `save_many`/`delete` 的注释）。
    async fn write_locked(&self, user_ns: &str, list: &Vec<HttpTts>) -> Result<(), AppError> {
        self.docs.write_list(user_ns, DOC_NAME, list).await
    }
}

fn matches_target(tts: &HttpTts, id: i64, name: &str) -> bool {
    (id != 0 && tts.id == id) || (!name.is_empty() && tts.name == name)
}

/// 给新引擎分配 id：毫秒时间戳，撞了就 +1。
///
/// 同一批次里可能连续插入多条，而毫秒时间戳会相同；撞号检查必须在**已推入 list**
/// 的状态下做，所以调用点放在 `list.push` 之前、逐条计算。
fn next_id(list: &[HttpTts]) -> i64 {
    let mut candidate = chrono::Utc::now().timestamp_millis();
    while list.iter().any(|tts| tts.id == candidate) {
        candidate += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db;

    /// 用内存 SQLite：`JsonDocumentService` 的落盘都走数据库，不需要真实数据目录，
    /// 也就避开了「本机 %TEMP% 不可写」导致整组单测失败的问题。
    async fn create_service() -> HttpTtsService {
        let pool = db::init_pool("sqlite::memory:").await.unwrap();
        let docs = Arc::new(JsonDocumentService::new(
            pool,
            &format!("{}/__http_tts_service_no_legacy_dir", env!("CARGO_MANIFEST_DIR")),
        ));
        HttpTtsService::new(docs)
    }

    fn tts(id: i64, name: &str, url: &str) -> HttpTts {
        HttpTts {
            id,
            name: name.to_string(),
            url: url.to_string(),
            ..HttpTts::default()
        }
    }

    #[tokio::test]
    async fn upserts_by_name_and_keeps_local_id() {
        let service = create_service().await;
        service.save_many("default", vec![tts(1, "A", "https://a.test")]).await.unwrap();
        // 同名再来一条（远程集合里 id 不同）：覆盖内容但保留本地 id=1。
        let summary = service
            .save_many("default", vec![tts(99, "A", "https://a2.test")])
            .await
            .unwrap();
        assert_eq!(summary, SaveSummary { saved: 1, skipped: 0 });
        let list = service.list("default").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, 1);
        assert_eq!(list[0].url, "https://a2.test");
    }

    #[tokio::test]
    async fn upserts_by_id_when_name_is_new() {
        let service = create_service().await;
        service.save_many("default", vec![tts(5, "A", "https://a.test")]).await.unwrap();
        // 名字改了、id 相同 —— 视为同一条引擎（改名），不是新增。
        service.save_many("default", vec![tts(5, "A2", "https://a.test")]).await.unwrap();
        let list = service.list("default").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "A2");
    }

    #[tokio::test]
    async fn assigns_timestamp_ids_when_missing() {
        let service = create_service().await;
        service
            .save_many(
                "default",
                vec![tts(0, "A", "https://a.test"), tts(0, "B", "https://b.test")],
            )
            .await
            .unwrap();
        let list = service.list("default").await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|tts| tts.id > 1_000_000_000_000));
        // 同一批次连续两条不能撞 id（毫秒时间戳会相同，必须 +1 避让）。
        assert_ne!(list[0].id, list[1].id);
    }

    #[tokio::test]
    async fn skips_entries_without_name_or_url() {
        let service = create_service().await;
        let summary = service
            .save_many(
                "default",
                vec![
                    tts(0, "", "https://a.test"),
                    tts(0, "B", "  "),
                    tts(0, "C", "https://c.test"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(summary, SaveSummary { saved: 1, skipped: 2 });
        assert_eq!(service.list("default").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn lists_by_name() {
        let service = create_service().await;
        service
            .save_many(
                "default",
                vec![tts(1, "B", "https://b.test"), tts(2, "A", "https://a.test")],
            )
            .await
            .unwrap();
        let names: Vec<String> = service
            .list("default")
            .await
            .unwrap()
            .into_iter()
            .map(|tts| tts.name)
            .collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn finds_by_id_or_name() {
        let service = create_service().await;
        service.save_many("default", vec![tts(42, "A", "https://a.test")]).await.unwrap();
        assert_eq!(service.find("default", "42").await.unwrap().unwrap().name, "A");
        assert_eq!(service.find("default", "A").await.unwrap().unwrap().id, 42);
        assert!(service.find("default", "nope").await.unwrap().is_none());
        assert!(service.find("default", " ").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn deletes_by_id_or_name() {
        let service = create_service().await;
        service
            .save_many(
                "default",
                vec![tts(1, "A", "https://a.test"), tts(2, "B", "https://b.test")],
            )
            .await
            .unwrap();
        assert_eq!(service.delete("default", 1, "").await.unwrap(), 1);
        assert_eq!(service.delete("default", 0, "B").await.unwrap(), 1);
        assert_eq!(service.delete("default", 0, "B").await.unwrap(), 0);
        assert!(service.list("default").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deletes_many_in_one_pass() {
        let service = create_service().await;
        service
            .save_many(
                "default",
                vec![
                    tts(1, "A", "https://a.test"),
                    tts(2, "B", "https://b.test"),
                    tts(3, "C", "https://c.test"),
                ],
            )
            .await
            .unwrap();
        let deleted = service
            .delete_many("default", &[tts(1, "", ""), tts(0, "C", "")])
            .await
            .unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(service.list("default").await.unwrap()[0].name, "B");
    }

    /// 远端 `read_remote_http_tts_file` 拿到的是整份 JSON 文本；这里用同样的
    /// 形态走一遍 service，确认导入后能原样读出（id 保留、字段不丢）。
    #[tokio::test]
    async fn imports_remote_list_shape() {
        let service = create_service().await;
        let payload = serde_json::json!([
            {
                "concurrentRate": "0",
                "contentType": "",
                "enabledCookieJar": false,
                "header": "",
                "id": 1652146476375i64,
                "lastUpdateTime": 1695006617414i64,
                "loginCheckJs": "",
                "loginUi": "",
                "loginUrl": "",
                "name": "导入引擎",
                "url": "https://example.test/tts?text={{speakText}}"
            }
        ]);
        let items: Vec<HttpTts> = serde_json::from_value(payload).unwrap();
        let summary = service.save_many("default", items).await.unwrap();
        assert_eq!(summary.saved, 1);
        let list = service.list("default").await.unwrap();
        assert_eq!(list[0].id, 1652146476375);
        assert_eq!(list[0].concurrent_rate.as_deref(), Some("0"));
        assert_eq!(list[0].last_update_time, Some(1695006617414));
    }
}
