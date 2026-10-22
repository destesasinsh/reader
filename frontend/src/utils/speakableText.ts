// 「这段文本有没有可朗读的内容」判定。
//
// 为什么需要它：Edge / 微软朗读这类在线服务对**纯标点、纯引号**的文本不会返回音频
// （服务端正常行为，不是故障）。可网文里 `……` 经常单独成段，双音色把
// `他说：“……”` 切开后对话段也只剩引号 —— 这类块发出去只会换来错误/空音频，
// 继而把整段朗读打断。所以必须在切块阶段就丢掉，而不是等失败后再容错。
//
// 中文字符属于 Unicode 的 `\p{L}`（Letter），全角数字属于 `\p{N}`；空白、标点、
// 引号、破折号、省略号都不属于任何一类，正好符合「无可朗读内容」的定义。

/** 至少含一个字母或数字才算「有内容」；预编译正则避免每次切块重复构造。 */
const SPEAKABLE_CONTENT_PATTERN = /[\p{L}\p{N}]/u

export function hasSpeakableContent(text: string): boolean {
  return SPEAKABLE_CONTENT_PATTERN.test(text)
}
