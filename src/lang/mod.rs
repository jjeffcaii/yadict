use anyhow::Result;
use encoding::label::encoding_from_whatwg_label;
use icu::collator::{Collator, CollatorOptions};
use icu::locid::Locale;
use log::__private_api::loc;
use std::cmp::Ordering;
use whatlang::{Lang, detect};

/// 第一步：自动判断输入的字符串样本主要属于什么语言，并转换为 ICU 的 Locale
fn detect_locale_from_text(text: &str) -> Locale {
    // 自动检测文本语言，如果检测失败或文本太短，默认降级为英文 "en"
    let detected_lang = detect(text).map(|info| info.lang()).unwrap_or(Lang::Eng);

    // 将检测到的语言转换为标准的 BCP-47 语言标签字符串
    let lang_code = match detected_lang {
        Lang::Cmn => "zh", // 对应 ICU4X 的中文
        Lang::Eng => "en", // 英文
        Lang::Jpn => "ja", // 日文
        Lang::Kor => "ko", // 韩文
        Lang::Deu => "de", // 德文
        _ => "en",         // 其他未知语言默认降级到英文
    };

    debug!(
        "-> 智能检测结果: 识别为 [{:?}], 匹配 ICU 标签: \"{}\"",
        detected_lang, lang_code
    );

    // 第二步：将字符串解析为 ICU4X 的标准 Locale 结构体
    lang_code.parse::<Locale>().unwrap_or_default()
}

pub fn compare(a: &str, b: &str) -> Ordering {
    let locale = detect_locale_from_text(a);
    let collator = Collator::try_new(&locale.into(), CollatorOptions::new()).unwrap();
    collator.compare(a, b)
}

pub fn decode(encoding: &str, raw: &[u8]) -> Result<String> {
    let decoder = encoding_from_whatwg_label(encoding)
        .ok_or_else(|| anyhow!("unknown encoding '{}'", encoding))?;
    decoder
        .decode(raw, encoding::DecoderTrap::Ignore)
        .map_err(|e| anyhow!("decode error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare() {
        // assert_eq!(Ordering::Less, compare("cat", "dog"));
        // assert_eq!(Ordering::Less, compare("cat", "Dog"));

        assert_eq!(Ordering::Less, compare("Benedictine", "bird"));
    }
}
