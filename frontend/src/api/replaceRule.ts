import { get, post } from './invoke'
import type { ReplaceRule } from '../types'

/**
 * 获取所有净化规则
 */
export function getReplaceRules() {
  return get<ReplaceRule[]>('/getReplaceRules').then((r) => r.data)
}

/**
 * 保存单个净化规则
 */
export function saveReplaceRule(rule: ReplaceRule) {
  return post<string>('/saveReplaceRule', rule).then((r) => r.data)
}

/**
 * 批量保存净化规则
 */
export function saveReplaceRules(rules: ReplaceRule[]) {
  return post<string>('/saveReplaceRules', rules).then((r) => r.data)
}

/**
 * 删除单个净化规则
 */
export function deleteReplaceRule(rule: ReplaceRule) {
  return post<string>('/deleteReplaceRule', rule).then((r) => r.data)
}

/**
 * 批量删除净化规则
 */
export function deleteReplaceRules(rules: ReplaceRule[]) {
  return post<string>('/deleteReplaceRules', rules).then((r) => r.data)
}

/** 送后端沙盒执行的规则步：正则已降级成 JS 语法。 */
export interface SandboxReplaceRuleStep {
  pattern: string
  flags: string
  replacement: string
  isScript: boolean
}

export interface SandboxReplaceRulesParam {
  text: string
  rules: SandboxReplaceRuleStep[]
}

/**
 * 在 Rust 沙盒里执行整条规则链（含 `@js:` 脚本规则）。
 *
 * 只有存在脚本规则时才调用；纯正则/字面量规则在前端同步执行，避免每章多一次 IPC。
 */
export function applyReplaceRulesInSandbox(params: SandboxReplaceRulesParam) {
  return post<string>('/applyReplaceRules', params).then((r) => r.data)
}
