use super::*;

pub fn metadata_from_value(value: Value) -> Metadata {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

pub(super) fn verify_diagnostic(severity: &str, message: String) -> CadDiagnostic {
    CadDiagnostic {
        severity: severity.to_string(),
        message,
        line: None,
        column: None,
    }
}

pub(super) fn source_hash(source: &str) -> String {
    storage::sha256_hex(source.as_bytes())
}

pub(super) fn json_to_parameter_value(value: Value) -> CadParameterValue {
    match value {
        Value::Bool(value) => CadParameterValue::Boolean(value),
        Value::Number(value) => CadParameterValue::Number(value.as_f64().unwrap_or_default()),
        Value::String(value) => CadParameterValue::String(value),
        other => CadParameterValue::String(other.to_string()),
    }
}

pub(super) fn propose_session_title(text: &str) -> Option<String> {
    let mut words = Vec::new();
    for raw_word in text.split(|character: char| !character.is_ascii_alphanumeric()) {
        let word = raw_word.trim().to_ascii_lowercase();
        if word.len() < 3 || TITLE_STOP_WORDS.contains(&word.as_str()) {
            continue;
        }
        if words.iter().any(|existing| existing == &word) {
            continue;
        }
        words.push(word);
        if words.len() == 4 {
            break;
        }
    }
    if words.is_empty() {
        return None;
    }
    Some(
        words
            .into_iter()
            .map(|word| {
                let mut characters = word.chars();
                match characters.next() {
                    Some(first) => {
                        let mut titled = first.to_ascii_uppercase().to_string();
                        titled.push_str(characters.as_str());
                        titled
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

const TITLE_STOP_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "from",
    "into",
    "cad",
    "model",
    "make",
    "create",
    "build",
    "design",
    "generate",
    "please",
    "using",
    "parametric",
    "openscad",
];

pub(super) fn uuid() -> String {
    Uuid::new_v4().to_string()
}

pub(super) fn timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}.{:03}Z", chrono_like_seconds(millis), millis % 1000)
}

fn chrono_like_seconds(millis: u128) -> String {
    let seconds = millis / 1000;
    let tm = time_from_unix(seconds as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second
    )
}

struct SimpleUtcTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn time_from_unix(seconds: i64) -> SimpleUtcTime {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    SimpleUtcTime {
        year,
        month,
        day,
        hour: seconds_of_day / 3600,
        minute: seconds_of_day % 3600 / 60,
        second: seconds_of_day % 60,
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

pub(super) fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "Session service lock is poisoned.".to_string()
}
