//! Data rules (ADR 0086): what a result may not carry to an agent, such as
//! location. These detectors refuse rather than redact, and refuse what
//! they can't read (binary, or long encoded runs), but they can miss what
//! they don't recognise: only a rule naming the fields allowed, or a source
//! that never holds the data, is a guarantee.

use serde::{Deserialize, Serialize};

/// A kind of data a rule keeps from agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Location,
}

impl DataClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Location => "location",
        }
    }
}

/// Why a result was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub class: DataClass,
    /// What was found, without the data itself.
    pub what: String,
}

/// Words that name location, in field and key names split into words.
const LOCATION_WORDS: [&str; 16] = [
    "lat",
    "lng",
    "lon",
    "latitude",
    "longitude",
    "location",
    "locations",
    "gps",
    "geo",
    "geopoint",
    "geohash",
    "coordinates",
    "coords",
    "position",
    "positions",
    "geometry",
];

/// `name` split into lower-case words at `_`, `-`, `.` and case changes.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for character in name.chars() {
        if !character.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = false;
            continue;
        }
        if character.is_uppercase() && previous_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        previous_lower = character.is_lowercase() || character.is_ascii_digit();
        word.extend(character.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn names_location(name: &str) -> bool {
    words(name).iter().any(|word| LOCATION_WORDS.contains(&word.as_str()) || word.starts_with("geo"))
}

/// Fields a GraphQL request selects that a rule refuses before it's sent.
pub fn graphql_fields<'a>(class: DataClass, fields: impl IntoIterator<Item = &'a String>) -> Option<Finding> {
    match class {
        DataClass::Location => {
            let named: Vec<&str> = fields.into_iter().filter(|field| names_location(field)).map(String::as_str).collect();
            (!named.is_empty()).then(|| Finding { class, what: format!("it selects {}", named.join(", ")) })
        }
    }
}

/// Whether a JSON result carries `class`: by its keys, and by its strings
/// as [`text`] reads them.
pub fn json(class: DataClass, value: &serde_json::Value) -> Option<Finding> {
    match value {
        serde_json::Value::Object(map) => map.iter().find_map(|(key, value)| {
            if !value.is_null() && class == DataClass::Location && names_location(key) {
                return Some(Finding { class, what: format!("it holds `{key}`") });
            }
            json(class, value)
        }),
        serde_json::Value::Array(items) => items.iter().find_map(|item| json(class, item)),
        serde_json::Value::String(string) => text(class, string),
        _ => None,
    }
}

/// Whether text output carries `class`, or can't be read for it.
pub fn text(class: DataClass, output: &str) -> Option<Finding> {
    let found = |what: &str| Some(Finding { class, what: what.to_owned() });
    if output.contains('\0') || output.chars().filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x1b')).count() > 8
    {
        return found("it's binary, which can't be checked");
    }
    if encoded_run(output) {
        return found("it holds a long encoded run, which can't be checked");
    }
    match class {
        DataClass::Location => {
            let upper = output.to_ascii_uppercase();
            if ["$GPGGA", "$GNGGA", "$GPRMC", "$GNRMC", "$GPGLL", "$GNGLL", "$GPGNS", "$GNGNS", "GPGGA ", "GNGGA "]
                .iter()
                .any(|sentence| upper.contains(sentence))
            {
                return found("it holds NMEA position sentences");
            }
            if output.contains("\"coordinates\"") || output.contains("<trkpt") || output.contains("<coordinates>") {
                return found("it holds GeoJSON, GPX or KML coordinates");
            }
            if let Some(what) = coordinates(output) {
                return found(&what);
            }
            None
        }
    }
}

/// A run of 200 or more base64 or hex characters.
fn encoded_run(output: &str) -> bool {
    let mut run = 0;
    for byte in output.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_') {
            run += 1;
            if run >= 200 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Numbers in `output`, with where each ends and how many decimals it has.
fn numbers(output: &str) -> Vec<(usize, usize, f64, usize)> {
    let bytes = output.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let starts =
            bytes[index].is_ascii_digit() || (bytes[index] == b'-' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit));
        let after_word = index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'.');
        if !starts || after_word {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            index += 1;
        }
        let text = &output[start..index];
        if let Ok(value) = text.parse::<f64>() {
            let decimals = text.split_once('.').map_or(0, |(_, after)| after.len());
            found.push((start, index, value, decimals));
        }
    }
    found
}

/// A latitude and longitude next to each other, or a number right after a
/// word that names one.
fn coordinates(output: &str) -> Option<String> {
    let numbers = numbers(output);
    for pair in numbers.windows(2) {
        let ((_, end, lat, lat_places), (start, _, lon, lon_places)) = (pair[0], pair[1]);
        let between = &output[end..start];
        if lat_places >= 4
            && lon_places >= 4
            && between.len() <= 3
            && between.chars().all(|c| matches!(c, ',' | ' ' | ';' | '\t'))
            && lat.abs() <= 90.0
            && lon.abs() <= 180.0
            // Timings and ratios sit near 0; places almost never do.
            && (lat.abs() >= 1.0 || lon.abs() >= 1.0)
        {
            return Some("it holds a latitude and longitude pair".into());
        }
    }
    let lower = output.to_ascii_lowercase();
    for (start, _, _, places) in numbers {
        let before = &lower[start.saturating_sub(14)..start];
        let named = ["lat", "lng", "lon", "latitude", "longitude"].iter().any(|word| {
            before.rfind(word).is_some_and(|at| {
                let gap = &before[at + word.len()..];
                gap.len() <= 4 && gap.chars().all(|c| matches!(c, ' ' | ':' | '=' | '"' | '\''))
            })
        });
        if named && (places >= 2 || output[start..].chars().take_while(char::is_ascii_digit).count() >= 7) {
            return Some("it holds a latitude or longitude".into());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: DataClass = DataClass::Location;

    #[test]
    fn location_is_found_in_the_forms_it_comes_in() {
        for output in [
            "car-0142 at 59.329323, 18.068581 since 23:41",
            "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47",
            "lat=59.3293 lon=18.0686",
            "\"lat\": 593293230, \"lon\": 180685810",
            "{\"type\":\"Point\",\"coordinates\":[18.06,59.33]}",
            "<trkpt lat=\"59.3\" lon=\"18.0\">",
        ] {
            assert!(text(L, output).is_some(), "{output}");
        }
        assert!(json(L, &serde_json::json!({"car": {"id": 1, "lastPosition": {"x": 1}}})).is_some());
        assert!(json(L, &serde_json::json!({"car": {"id": 1, "geoHash": "u6sce"}})).is_some());
    }

    #[test]
    fn ordinary_output_passes() {
        for output in [
            "Nov 16 23:41:09 gpsd[611]: lost fix: antenna current 0 mA",
            "deploy/telemetry 3/3 ready, version 1.24.3, 12.5% cpu",
            "took 0.0042, 0.0031 seconds",
            "commit 4f2a9c1 Add retry with backoff",
        ] {
            assert_eq!(text(L, output), None, "{output}");
        }
        assert_eq!(json(L, &serde_json::json!({"car": {"id": 1, "otaEnabled": true, "location": null}})), None);
    }

    #[test]
    fn what_cant_be_read_is_refused() {
        assert!(text(L, "ok\0\u{1}\u{2}").is_some());
        assert!(text(L, &"QUJD".repeat(60)).is_some(), "base64 hides anything");
    }

    #[test]
    fn graphql_fields_by_name() {
        let fields: Vec<String> = ["vehicle", "id", "lastKnownPosition", "otaEnabled"].map(str::to_owned).into();
        assert_eq!(graphql_fields(L, &fields).unwrap().what, "it selects lastKnownPosition");
        let plain: Vec<String> = ["vehicle", "id", "otaChannel"].map(str::to_owned).into();
        assert_eq!(graphql_fields(L, &plain), None);
    }
}
