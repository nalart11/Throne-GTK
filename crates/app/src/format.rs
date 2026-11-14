//! Unified number formatting: the same representation in the list, status, and
//! connections table.

use throne_config::settings::Language;

pub fn bytes(value: i64) -> String {
    bytes_for(value, crate::i18n::language())
}

fn bytes_for(value: i64, language: Language) -> String {
    let units: [&str; 5] = match language {
        Language::Ru => ["Б", "КБ", "МБ", "ГБ", "ТБ"],
        Language::En => ["B", "KB", "MB", "GB", "TB"],
    };
    let mut v = value.max(0) as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < units.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value.max(0), units[0])
    } else if v < 10.0 {
        format!("{v:.1} {}", units[unit])
    } else {
        format!("{v:.0} {}", units[unit])
    }
}

pub fn rate(bytes_per_second: i64) -> String {
    format!("{}{}", bytes(bytes_per_second), crate::i18n::tr("/с", "/s"))
}

/// Latency and its styling class. The boundaries reflect perceived responsiveness:
/// interactive work does not notice latency below 100 ms, while after 300 ms even
/// page loading notices it.
pub fn latency(ms: i32) -> (String, &'static str) {
    match ms {
        0 => ("—".into(), ""),
        ms if ms < 0 => (crate::i18n::tr("нет связи", "unreachable").into(), "slow"),
        ms if ms < 100 => (crate::tr_format!("{ms} мс", "{ms} ms"), "fast"),
        ms if ms < 300 => (crate::tr_format!("{ms} мс", "{ms} ms"), "medium"),
        ms => (crate::tr_format!("{ms} мс", "{ms} ms"), "slow"),
    }
}

/// How much time has passed since a Unix timestamp.
pub fn since(unix: i64) -> String {
    since_for(unix, crate::i18n::language())
}

fn since_for(unix: i64, language: Language) -> String {
    if unix <= 0 {
        return match language {
            Language::Ru => "никогда",
            Language::En => "never",
        }
        .into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = (now - unix).max(0);
    match (language, delta) {
        (Language::Ru, 0..=59) => "только что".into(),
        (Language::En, 0..=59) => "just now".into(),
        (Language::Ru, 60..=3599) => format!("{} мин назад", delta / 60),
        (Language::En, 60..=3599) => format!("{} min ago", delta / 60),
        (Language::Ru, 3600..=86399) => format!("{} ч назад", delta / 3600),
        (Language::En, 3600..=86399) => format!("{} h ago", delta / 3600),
        (Language::Ru, _) => format!("{} дн назад", delta / 86400),
        (Language::En, _) => format!("{} d ago", delta / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_switch_units() {
        assert_eq!(bytes_for(1024, Language::En), "1.0 KB");
        assert_eq!(bytes_for(0, Language::Ru), "0 Б");
        assert_eq!(bytes_for(512, Language::Ru), "512 Б");
        assert_eq!(bytes_for(1024, Language::Ru), "1.0 КБ");
        assert_eq!(bytes_for(20 * 1024, Language::Ru), "20 КБ");
        assert_eq!(bytes_for(5 * 1024 * 1024, Language::Ru), "5.0 МБ");
    }

    #[test]
    fn latency_classes() {
        assert_eq!(latency(0).0, "—");
        assert_eq!(latency(-1).1, "slow");
        assert_eq!(latency(42).1, "fast");
        assert_eq!(latency(150).1, "medium");
        assert_eq!(latency(900).1, "slow");
    }

    #[test]
    fn never_tested_reads_as_never() {
        assert_eq!(since_for(0, Language::Ru), "никогда");
        assert_eq!(since_for(0, Language::En), "never");
    }
}
