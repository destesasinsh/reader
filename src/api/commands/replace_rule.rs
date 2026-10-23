use crate::api::AppState;
use serde_json::Value;

use crate::error::error::{ApiResponse, AppError};
use crate::model::replace_rule::ReplaceRule;

/// `get_replace_rules` 的共享实现 (Tauri command 与 Web HTTP handler 共用)。
pub async fn get_replace_rules_core(state: &AppState) -> Result<ApiResponse<Value>, AppError> {
    let user_ns = "default";
    let list = read_list::<ReplaceRule>(state, &user_ns, "replaceRule.json").await?;
    Ok(ApiResponse::ok(
        serde_json::to_value(list).unwrap_or_default(),
    ))
}

/// `get_replace_rules` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn get_replace_rules(
    state: tauri::State<'_, AppState>,
) -> Result<ApiResponse<Value>, AppError> {
    get_replace_rules_core(&state).await
}

/// `save_replace_rule` 的共享实现。
///
/// 阅读页的「按本书替换 / 按书源替换」会从这里写入规则, Web 端同样需要 ——
/// 否则划词菜单里那两个按钮点了只会 404。
pub async fn save_replace_rule_core(
    state: &AppState,
    req: ReplaceRule,
) -> Result<ApiResponse<Value>, AppError> {
    let user_ns = "default";
    if req.name.is_empty() {
        return Err(AppError::BadRequest("名称不能为空".to_string()));
    }
    if req.pattern.is_empty() {
        return Err(AppError::BadRequest("替换规则不能为空".to_string()));
    }
    let mut list = read_list::<ReplaceRule>(state, &user_ns, "replaceRule.json").await?;
    upsert_by_key(&mut list, req, |r| r.name.clone());
    write_list(state, &user_ns, "replaceRule.json", &list).await?;
    Ok(ApiResponse::ok(Value::String("".to_string())))
}

#[tauri::command]
pub async fn save_replace_rule(
    state: tauri::State<'_, AppState>,
    req: ReplaceRule,
) -> Result<ApiResponse<Value>, AppError> {
    save_replace_rule_core(&state, req).await
}

/// `save_replace_rules` 的共享实现 (批量 upsert)。
pub async fn save_replace_rules_core(
    state: &AppState,
    mut req: Vec<ReplaceRule>,
) -> Result<ApiResponse<Value>, AppError> {
    let user_ns = "default";
    let mut list = read_list::<ReplaceRule>(state, &user_ns, "replaceRule.json").await?;
    req.retain(|r| !r.name.is_empty() && !r.pattern.is_empty());
    for r in req {
        upsert_by_key(&mut list, r, |v| v.name.clone());
    }
    write_list(state, &user_ns, "replaceRule.json", &list).await?;
    Ok(ApiResponse::ok(Value::String("".to_string())))
}

#[tauri::command]
pub async fn save_replace_rules(
    state: tauri::State<'_, AppState>,
    req: Vec<ReplaceRule>,
) -> Result<ApiResponse<Value>, AppError> {
    save_replace_rules_core(&state, req).await
}

/// `delete_replace_rule` 的共享实现。
pub async fn delete_replace_rule_core(
    state: &AppState,
    req: ReplaceRule,
) -> Result<ApiResponse<Value>, AppError> {
    let user_ns = "default";
    let mut list = read_list::<ReplaceRule>(state, &user_ns, "replaceRule.json").await?;
    list.retain(|r| r.name != req.name);
    write_list(state, &user_ns, "replaceRule.json", &list).await?;
    Ok(ApiResponse::ok(Value::String("".to_string())))
}

#[tauri::command]
pub async fn delete_replace_rule(
    state: tauri::State<'_, AppState>,
    req: ReplaceRule,
) -> Result<ApiResponse<Value>, AppError> {
    delete_replace_rule_core(&state, req).await
}

/// `delete_replace_rules` 的共享实现 (批量)。
pub async fn delete_replace_rules_core(
    state: &AppState,
    req: Vec<ReplaceRule>,
) -> Result<ApiResponse<Value>, AppError> {
    let user_ns = "default";
    let mut list = read_list::<ReplaceRule>(state, &user_ns, "replaceRule.json").await?;
    for r in req {
        list.retain(|v| v.name != r.name);
    }
    write_list(state, &user_ns, "replaceRule.json", &list).await?;
    Ok(ApiResponse::ok(Value::String("".to_string())))
}

#[tauri::command]
pub async fn delete_replace_rules(
    state: tauri::State<'_, AppState>,
    req: Vec<ReplaceRule>,
) -> Result<ApiResponse<Value>, AppError> {
    delete_replace_rules_core(&state, req).await
}

async fn read_list<T: for<'de> serde::Deserialize<'de>>(
    state: &AppState,
    user_ns: &str,
    name: &str,
) -> Result<Vec<T>, AppError> {
    state.json_document_service.read_list(user_ns, name).await
}

async fn write_list<T: serde::Serialize>(
    state: &AppState,
    user_ns: &str,
    name: &str,
    list: &Vec<T>,
) -> Result<(), AppError> {
    state
        .json_document_service
        .write_list(user_ns, name, list)
        .await
}

fn upsert_by_key<T, F>(list: &mut Vec<T>, item: T, key_fn: F)
where
    F: Fn(&T) -> String,
{
    let key = key_fn(&item);
    if let Some(pos) = list.iter().position(|v| key_fn(v) == key) {
        list[pos] = item;
    } else {
        list.push(item);
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReplaceRulesParam {
    pub text: String,
    pub rules: Vec<crate::parser::replace_rule_js::ReplaceRuleStep>,
}

/// 整条规则链的执行结果（含 `@js:` 脚本规则）。
///
/// 为什么放后端：脚本要在 rquickjs 沙盒里跑，不能进渲染进程（见
/// `parser::replace_rule_js` 的说明）。前端只在**存在脚本规则**时才走这条命令，
/// 纯正则/字面量规则仍走本地同步快路径，避免每章都多一次 IPC。
///
/// 纯计算、不依赖 `AppState`，所以 core 不带 state —— Web 端若将来要在浏览器里
/// 执行脚本规则，可以直接复用这个 core，不必再复制一份实现。
pub async fn apply_replace_rules_core(
    req: ApplyReplaceRulesParam,
) -> Result<ApiResponse<String>, AppError> {
    let ApplyReplaceRulesParam { text, rules } = req;
    if rules.len() > MAX_REPLACE_SCRIPT_RULES {
        return Err(AppError::BadRequest(format!(
            "净化规则数量超出上限（{} 条）",
            MAX_REPLACE_SCRIPT_RULES
        )));
    }

    // 脚本是用户从网上导入的，可能写出死循环。两层保护缺一不可：
    // 1. 沙盒内部的 interrupt handler（`apply_rules_with_timeout`）真正打断执行，
    //    超时返回时线程已经停下，不会留下还在烧 CPU 的后台线程；
    // 2. 这里的 `timeout` 是兜底：万一 interrupt handler 没能生效（比如卡在
    //    Rust 侧的分配/拷贝），至少不会把 IPC 运行时拖住。
    let handle = tokio::task::spawn_blocking(move || {
        crate::parser::replace_rule_js::apply_rules_with_timeout(
            &text,
            &rules,
            REPLACE_SCRIPT_TIMEOUT,
        )
    });

    match tokio::time::timeout(REPLACE_SCRIPT_TIMEOUT + INTERRUPT_GRACE, handle).await {
        Ok(Ok(Ok(output))) => Ok(ApiResponse::ok(output)),
        Ok(Ok(Err(message))) => Err(AppError::BadRequest(message)),
        Ok(Err(join_error)) => Err(AppError::BadRequest(format!(
            "净化脚本线程异常: {join_error}"
        ))),
        Err(_) => Err(AppError::BadRequest(
            "净化脚本执行超时，可能存在死循环，已跳过本章净化".to_string(),
        )),
    }
}

/// 单章规则条数上限：正常规则集是几十条，几百条已属异常，防止一次 IPC 塞爆。
const MAX_REPLACE_SCRIPT_RULES: usize = 500;

/// 单章脚本执行上限。正常整章处理在毫秒级，超过这个量级基本可以判定是死循环。
const REPLACE_SCRIPT_TIMEOUT: std::time::Duration = crate::parser::replace_rule_js::DEFAULT_SCRIPT_TIMEOUT;

/// 外层 timeout 的宽限：内层 interrupt 到期后还要走完 unwind 与错误文案构造，
/// 外层若卡在同一时刻会偶发抢先报"超时"、丢掉内层更准确的原因。
const INTERRUPT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// `apply_replace_rules` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn apply_replace_rules(
    req: ApplyReplaceRulesParam,
) -> Result<ApiResponse<String>, AppError> {
    apply_replace_rules_core(req).await
}
