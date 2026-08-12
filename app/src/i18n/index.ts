// 界面文案的取用。零依赖。
//
// 规范：openspec/changes/add-steadcopy-i18n/specs/i18n/spec.md
//
// **不引 i18n 库。** 需求是两种语言的静态词典，一个几十行的 `t()` 就够；
// 引库反而要为它的加载时机、命名空间、复数规则付出理解成本。
//
// 复数：中文没有复数变化，英文里能绕开就绕开（写 "3 file(s)" 而不是搞一套规则）。

import { en } from "./en";
import { zh, type Key } from "./zh";

export type { Key };
export type Lang = "zh" | "en";

const TABLES: Record<Lang, Record<Key, string>> = { zh, en };

/** 当前语言。由 App 在读到配置后设一次，之后只读。 */
let current: Lang = "zh";

export function setLang(l: Lang) {
  current = l;
}

export function lang(): Lang {
  return current;
}

/**
 * 取一条文案，`{name}` 形式插值。
 *
 * 没有「查不到就返回键名/空串」的兜底——`Key` 是从中文词典推导的，
 * 传不进去一个不存在的键；`en.ts` 是 `Record<Key, string>`，少一条编译就红。
 * 兜底在这里是多余的，而多余的兜底会掩盖真正的漏译。
 */
export function t(key: Key, params?: Record<string, string | number>): string {
  const s = TABLES[current][key];
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) =>
    k in params ? String(params[k]) : m
  );
}

/** 由配置里的 `auto` / `zh` / `en` 与浏览器语言定出实际语言。 */
export function resolveLang(setting: string): Lang {
  if (setting === "en") return "en";
  if (setting === "zh") return "zh";
  // auto：跟系统。判不出来落中文——第一受众是中文创作者，
  // 判不出来给中文是更安全的猜测，也绝不给空白
  return navigator.language?.toLowerCase().startsWith("en") ? "en" : "zh";
}
