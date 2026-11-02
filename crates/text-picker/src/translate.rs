//! 翻译：语言判定、请求 URL、响应解析、浏览器兜底。
//!
//! 翻译使用无需 API Key 的公共接口，
//! 失败时退化为"用默认浏览器打开 Bing 翻译"。

use std::time::Duration;

/// 单次翻译的最大字符数（公共接口对匿名请求限制较严）。
pub const MAX_QUERY_CHARS: usize = 480;

/// 判断原文语言，返回 `(source, target)`。
pub fn language_pair(text: &str) -> (&'static str, &'static str) {
    let mut cjk = 0usize;
    let mut total = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        total += 1;
        let c = ch as u32;
        if (0x4E00..=0x9FFF).contains(&c) || (0x3400..=0x4DBF).contains(&c) {
            cjk += 1;
        }
    }
    if total > 0 && cjk * 5 >= total * 2 {
        ("zh-CN", "en")
    } else {
        ("en", "zh-CN")
    }
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for &b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 限制长度（按字符计数），避免超过公共接口上限。
pub fn truncate_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

pub fn api_url(text: &str, source: &str, target: &str) -> String {
    let q = percent_encode(&truncate_chars(text, MAX_QUERY_CHARS));
    let pair = percent_encode(&format!("{}|{}", source, target));
    format!("https://api.mymemory.translated.net/get?q={}&langpair={}", q, pair)
}

/// 浏览器兜底地址（Bing 翻译，国内可访问性较好）。
pub fn browser_url(text: &str, _source: &str, target: &str) -> String {
    let q = percent_encode(&truncate_chars(text, MAX_QUERY_CHARS));
    let to = if target == "en" { "en" } else { "zh-Hans" };
    format!("https://www.bing.com/translator?text={}&from=auto&to={}", q, to)
}

/// 极简 JSON 字符串取值：从 `"key":"value"` 中取出并反转义 value。
/// 只服务于本项目的固定响应格式，避免为此引入完整 JSON 依赖。
pub fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    let mut unicode_buf = String::new();
    let mut in_unicode = false;
    for (_, ch) in chars {
        if in_unicode {
            unicode_buf.push(ch);
            if unicode_buf.len() == 4 {
                if let Ok(code) = u32::from_str_radix(&unicode_buf, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
                unicode_buf.clear();
                in_unicode = false;
            }
            continue;
        }
        if escaped {
            escaped = false;
            match ch {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => in_unicode = true,
                other => out.push(other),
            }
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

/// MyMemory 会把 `'` 之类编码成 HTML 实体。
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let (decoded, consumed) = if let Some(end) = tail.find(';') {
            let entity = &tail[1..end];
            let replacement = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => {
                    if let Some(num) = entity.strip_prefix('#') {
                        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            num.parse::<u32>().ok()
                        };
                        code.and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            match replacement {
                Some(c) => (Some(c), end + 1),
                None => (None, 1),
            }
        } else {
            (None, 1)
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[consumed..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 解析固定位置的非字符串数字字段（如 `"responseStatus":403`）。
fn json_number_field(json: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{}\"", key);
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// 解析翻译接口响应。返回 `Err` 时附带可直接展示的中文原因。
pub fn parse_response(body: &str) -> Result<String, String> {
    if json_number_field(body, "responseStatus").map(|s| s != 200).unwrap_or(false) {
        // 配额/限流等情况会在 responseDetails 中给出原因。
        if let Some(msg) = json_string_field(body, "responseDetails") {
            if !msg.is_empty() && msg != "''" {
                return Err(decode_entities(&msg));
            }
        }
        return Err("翻译服务返回错误".to_string());
    }
    let text = json_string_field(body, "translatedText")
        .ok_or_else(|| "翻译服务返回了无法解析的内容".to_string())?;
    let text = decode_entities(&text);
    if text.trim().is_empty() {
        return Err("翻译结果为空".to_string());
    }
    Ok(text)
}

/// 同步翻译（在工作线程中调用）。
pub fn translate_blocking(text: &str) -> Result<String, String> {
    let (source, target) = language_pair(text);
    let url = api_url(text, source, target);
    let response = ureq::get(&url)
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("翻译服务返回 HTTP {}", code),
            ureq::Error::Transport(t) => format!("网络错误：{}", t),
        })?;
    let body = response
        .into_string()
        .map_err(|e| format!("读取翻译结果失败：{}", e))?;
    parse_response(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_source() {
        assert_eq!(language_pair("这是一段中文，需要翻译成英文。"), ("zh-CN", "en"));
        assert_eq!(language_pair("hello world"), ("en", "zh-CN"));
    }

    #[test]
    fn builds_encoded_api_url() {
        let url = api_url("hello world & 你好", "en", "zh-CN");
        assert!(url.contains("q=hello%20world%20%26%20%E4%BD%A0%E5%A5%BD"));
        assert!(url.contains("langpair=en%7Czh-CN"));
    }

    #[test]
    fn truncates_long_input_with_ellipsis() {
        let long = "a".repeat(MAX_QUERY_CHARS + 10);
        let out = truncate_chars(&long, MAX_QUERY_CHARS);
        assert_eq!(out.chars().count(), MAX_QUERY_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn parses_translated_text_and_escapes() {
        let body = r#"{"responseData":{"translatedText":"你好世界"},"responseStatus":200}"#;
        assert_eq!(parse_response(body).unwrap(), "你好世界");

        let body = r#"{"responseData":{"translatedText":"it\u0027s \u4f60\u597d"}}"#;
        assert_eq!(parse_response(body).unwrap(), "it's 你好");
    }

    #[test]
    fn decodes_html_entities() {
        assert_eq!(decode_entities("it&#39;s &amp; &quot;ok&quot;"), "it's & \"ok\"");
        assert_eq!(decode_entities("a &unknown; b"), "a &unknown; b");
    }

    #[test]
    fn surfaces_service_error() {
        let body = r#"{"responseData":{"translatedText":"PLEASE SELECT TWO DISTINCT LANGUAGES"},"responseStatus":403,"responseDetails":"QUERY LENGTH LIMIT EXCEEDED"}"#;
        assert!(parse_response(body).is_err());
    }

    #[test]
    fn browser_url_uses_bing_translator() {
        let url = browser_url("hello", "en", "zh-CN");
        assert!(url.starts_with("https://www.bing.com/translator?text=hello"));
        assert!(url.contains("to=zh-Hans"));
    }
}
