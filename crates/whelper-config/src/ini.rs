//! 极简 INI 解析：`[section]` + `key = value`，支持 `;` / `#` 注释与 UTF-8 BOM。
//!
//! 只服务本项目的手写配置文件：不做类型推断、不做转义，未知内容原样保留在表里。

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Ini {
    sections: HashMap<String, HashMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Self::default();
        let mut section = String::new();

        for raw in text.trim_start_matches('\u{feff}').lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    section = name.trim().to_ascii_lowercase();
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            if key.is_empty() {
                continue;
            }
            ini.sections
                .entry(section.clone())
                .or_default()
                .insert(key, unquote(strip_comment(value.trim())).to_string());
        }
        ini
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_ascii_lowercase())
            .and_then(|items| items.get(&key.to_ascii_lowercase()))
            .map(String::as_str)
    }
}

/// 去掉值后面的 ` ; 注释` / ` # 注释`（只认前面有空白的注释符，避免误伤值本身）。
fn strip_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if (*byte == b';' || *byte == b'#')
            && (index == 0 || matches!(bytes[index - 1], b' ' | b'\t'))
        {
            return value[..index].trim_end();
        }
    }
    value
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_and_keys() {
        let ini = Ini::parse("[General]\nTheme = Dark\n[text-picker]\nenabled=true\n");
        assert_eq!(ini.get("general", "theme"), Some("Dark"));
        assert_eq!(ini.get("TEXT-PICKER", "ENABLED"), Some("true"));
        assert_eq!(ini.get("general", "enabled"), None);
    }

    #[test]
    fn ignores_comments_blank_lines_and_bom() {
        let ini = Ini::parse("\u{feff}; 头部注释\n\n[general]\n# 主题\ntheme = light ; 行尾注释\n");
        assert_eq!(ini.get("general", "theme"), Some("light"));
    }

    #[test]
    fn strips_quotes() {
        let ini = Ini::parse("[general]\npath = \"C:\\Users\\me\"\n");
        assert_eq!(ini.get("general", "path"), Some("C:\\Users\\me"));
    }

    #[test]
    fn last_value_wins() {
        let ini = Ini::parse("[general]\ntheme = light\ntheme = dark\n");
        assert_eq!(ini.get("general", "theme"), Some("dark"));
    }

    #[test]
    fn tolerates_broken_lines() {
        let ini = Ini::parse("[general]\ntheme = dark\n= nokey\nnoequals\n[broken\n");
        assert_eq!(ini.get("general", "theme"), Some("dark"));
    }

    #[test]
    fn comment_marker_inside_value_is_kept() {
        // 没有空白前缀的分号不是注释（避免把路径里的字符吃掉）。
        let ini = Ini::parse("[general]\npath = C:\\a;b\n");
        assert_eq!(ini.get("general", "path"), Some("C:\\a;b"));
    }
}
