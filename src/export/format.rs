//! Desktop's value formatting, with an explicit timezone context for repeatable exports.
use super::error::{ExportError, Result};
use jiff::{
    Timestamp, ZonedDateTime,
    tz::{Offset, TimeZone},
};
#[derive(Clone)]
pub struct DateContext {
    zone: TimeZone,
    standard_offset: i32,
}
impl Default for DateContext {
    fn default() -> Self {
        Self::system()
    }
}
impl DateContext {
    pub fn utc() -> Self {
        Self {
            zone: TimeZone::UTC,
            standard_offset: 0,
        }
    }
    pub fn fixed(seconds: i32) -> Result<Self> {
        Ok(Self {
            zone: TimeZone::fixed(
                Offset::from_seconds(seconds).map_err(|_| ExportError::InvalidOptions)?,
            ),
            standard_offset: seconds,
        })
    }
    pub fn named(name: &str) -> Result<Self> {
        let zone = TimeZone::get(name).map_err(|_| ExportError::InvalidOptions)?;
        Ok(Self::from_zone(zone))
    }
    pub fn system() -> Self {
        Self::from_zone(TimeZone::system())
    }
    fn from_zone(zone: TimeZone) -> Self {
        let now = Timestamp::now().to_zoned_datetime(zone.clone());
        let jan = now
            .with()
            .month(1)
            .day(1)
            .build()
            .unwrap_or_else(|_| now.clone())
            .offset()
            .seconds();
        let jul = now
            .with()
            .month(7)
            .day(1)
            .build()
            .unwrap_or_else(|_| now.clone())
            .offset()
            .seconds();
        Self {
            zone,
            standard_offset: jan.min(jul),
        }
    }
    pub fn datetime(&self, date: i64) -> Result<ZonedDateTime> {
        Ok(Timestamp::from_second(date)
            .map_err(|_| ExportError::InvalidRecord)?
            .to_zoned_datetime(self.zone.clone()))
    }
    pub fn standard_offset(&self) -> i32 {
        self.standard_offset
    }
}
pub fn fill_left(data: &str, length: usize, filler: &str) -> String {
    let count = data.chars().count();
    if length <= count {
        data.into()
    } else {
        filler.repeat(length - count) + data
    }
}
pub trait DesktopNumber {
    fn desktop_number(self) -> String;
}
macro_rules! integer_numbers {($($t:ty),*)=>{$(impl DesktopNumber for $t{fn desktop_number(self)->String{self.to_string()}})*};}
integer_numbers!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
impl DesktopNumber for f64 {
    fn desktop_number(self) -> String {
        if self.is_nan() {
            "nan".into()
        } else {
            format!("{self:.6}")
        }
    }
}
impl DesktopNumber for f32 {
    fn desktop_number(self) -> String {
        if self.is_nan() {
            "nan".into()
        } else {
            format!("{self:.6}")
        }
    }
}
pub fn number_to_string(value: impl DesktopNumber, length: usize, filler: &str) -> String {
    fill_left(&value.desktop_number(), length, filler).replace(',', ".")
}
pub fn iso_offset(offset: i32) -> String {
    let minutes = offset / 60;
    format!(
        "UTC{}{:02}:{:02}",
        if minutes >= 0 { '+' } else { '-' },
        minutes.unsigned_abs() / 60,
        minutes.unsigned_abs() % 60
    )
}
pub fn format_date_time(
    date: i64,
    has_time_zone: bool,
    date_separator: &str,
    time_separator: &str,
    separator: &str,
    context: &DateContext,
) -> Result<String> {
    if date == 0 {
        return Ok(String::new());
    }
    let value = context.datetime(date)?;
    let zone = if has_time_zone {
        format!("{separator}{}", iso_offset(context.standard_offset()))
    } else {
        String::new()
    };
    Ok(format!(
        "{:02}{date_separator}{:02}{date_separator}{}{separator}{:02}{time_separator}{:02}{time_separator}{:02}{zone}",
        value.day(),
        value.month(),
        value.year(),
        value.hour(),
        value.minute(),
        value.second()
    ))
}
pub fn iso_date_time(date: i64, context: &DateContext) -> Result<String> {
    let z = context.datetime(date)?;
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        z.year(),
        z.month(),
        z.day(),
        z.hour(),
        z.minute(),
        z.second()
    ))
}
pub fn format_file_size(size: u64) -> String {
    if size >= 1024 * 1024 {
        let tenth = u128::from(size) * 10 / (1024 * 1024);
        format!("{}.{:01} MB", tenth / 10, tenth % 10)
    } else if size >= 1024 {
        let tenth = u128::from(size) * 10 / 1024;
        format!("{}.{:01} KB", tenth / 10, tenth % 10)
    } else {
        format!("{size} B")
    }
}
pub fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
pub fn format_image_size_text(width: u32, height: u32) -> String {
    format!("{width}×{height}")
}
pub use super::format_money::{
    CurrencyRule, fill_amount_and_currency, format_grams_amount, format_money_amount,
    format_with_separators, lookup_currency_rule,
};
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CallingCodeInfo {
    pub calling_code: String,
    pub prefixes: Vec<String>,
    pub patterns: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CountryInfo {
    pub name: String,
    pub iso2: String,
    pub codes: Vec<CallingCodeInfo>,
    pub is_hidden: bool,
}
pub fn format_phone_number(phone: &str, countries: &[CountryInfo]) -> String {
    if phone.is_empty() || phone.starts_with('0') {
        return phone.into();
    }
    let phone = phone.replace(' ', "");
    let phone = phone.strip_prefix('+').unwrap_or(&phone);
    let mut best = None;
    let mut length = 0;
    for country in countries {
        for code in &country.codes {
            if let Some(rest) = phone.strip_prefix(&code.calling_code) {
                for prefix in &code.prefixes {
                    let size = code.calling_code.len() + prefix.len();
                    if size > length && rest.starts_with(prefix) {
                        best = Some(code);
                        length = size;
                    }
                }
            }
        }
    }
    let Some(best) = best else {
        return phone.into();
    };
    let part = &phone[best.calling_code.len()..];
    let mut formatted = part.to_owned();
    let mut max_matched = 0;
    for pattern in &best.patterns {
        let pattern = pattern.chars().collect::<Vec<_>>();
        let mut position = 0;
        let mut result = String::new();
        let mut matched = 0;
        let mut failed = false;
        for c in part.chars() {
            while position < pattern.len()
                && pattern[position] != 'X'
                && !pattern[position].is_ascii_digit()
            {
                result.push(pattern[position]);
                position += 1;
            }
            if position >= pattern.len() || pattern[position] == 'X' {
                position += 1;
                result.push(c);
            } else if c == pattern[position] {
                matched += 1;
                position += 1;
                result.push(c);
            } else {
                failed = true;
                break;
            }
        }
        if !failed && matched >= max_matched {
            max_matched = matched;
            formatted = result;
        }
    }
    format!("+{} {formatted}", best.calling_code)
}

pub fn countries_from_tl(result: &serde_json::Value) -> Vec<CountryInfo> {
    super::model::array(result, "countries")
        .iter()
        .filter(|c| !super::model::boolean(c, "hidden"))
        .map(|country| CountryInfo {
            name: super::model::string(country, "default_name").into(),
            iso2: super::model::string(country, "iso2").into(),
            is_hidden: false,
            codes: super::model::array(country, "country_codes")
                .iter()
                .map(|code| CallingCodeInfo {
                    calling_code: super::model::string(code, "country_code").into(),
                    prefixes: if code["prefixes"].is_null() {
                        vec![String::new()]
                    } else {
                        super::model::array(code, "prefixes")
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    },
                    patterns: super::model::array(code, "patterns")
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                })
                .collect(),
        })
        .collect()
}
