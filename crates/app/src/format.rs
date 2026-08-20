//! Единое форматирование чисел: то же представление в списке, в статусе и в
//! таблице соединений.

pub fn bytes(value: i64) -> String {
    const UNITS: [&str; 5] = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let mut v = value.max(0) as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value.max(0), UNITS[0])
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

pub fn rate(bytes_per_second: i64) -> String {
    format!("{}/с", bytes(bytes_per_second))
}

/// Задержка и класс оформления к ней. Границы взяты по ощущению отклика:
/// до 100 мс интерактивная работа не замечает задержки, после 300 мс её
/// замечает даже загрузка страницы.
pub fn latency(ms: i32) -> (String, &'static str) {
    match ms {
        0 => ("—".into(), ""),
        ms if ms < 0 => ("нет связи".into(), "slow"),
        ms if ms < 100 => (format!("{ms} мс"), "fast"),
        ms if ms < 300 => (format!("{ms} мс"), "medium"),
        ms => (format!("{ms} мс"), "slow"),
    }
}

/// Сколько времени прошло с момента в unix-времени.
pub fn since(unix: i64) -> String {
    if unix <= 0 {
        return "никогда".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = (now - unix).max(0);
    match delta {
        0..=59 => "только что".into(),
        60..=3599 => format!("{} мин назад", delta / 60),
        3600..=86399 => format!("{} ч назад", delta / 3600),
        _ => format!("{} дн назад", delta / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_switch_units() {
        assert_eq!(bytes(0), "0 Б");
        assert_eq!(bytes(512), "512 Б");
        assert_eq!(bytes(1024), "1.0 КБ");
        assert_eq!(bytes(20 * 1024), "20 КБ");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 МБ");
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
        assert_eq!(since(0), "никогда");
    }
}
