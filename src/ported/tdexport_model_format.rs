//! Desktop export formatting, compared against the frozen Python implementation.

use std::cmp::Ordering;

use jiff::{Timestamp, tz::TimeZone};
use serde_json::{Value, json};

pub fn fill_left(data: String, length: i64, filler: String) -> Result<String, String> {
    let count = usize::try_from(length)
        .unwrap_or(0)
        .saturating_sub(data.chars().count());
    if count
        .checked_mul(filler.chars().count())
        .is_none_or(|n| n > isize::MAX as usize)
    {
        return Err("OverflowError: repeated string is too long".into());
    }
    let Some(size) = count
        .checked_mul(filler.len())
        .and_then(|n| n.checked_add(data.len()))
    else {
        return Err("MemoryError: ".into());
    };
    let mut out = String::new();
    out.try_reserve_exact(size)
        .map_err(|_| "MemoryError: ".to_owned())?;
    out.push_str(&filler.repeat(count));
    out.push_str(&data);
    Ok(out)
}

fn fixed(value: f64, precision: usize) -> String {
    if value.is_nan() {
        "nan".into()
    } else {
        format!("{value:.precision$}")
    }
}

pub fn number_to_string(value: Value, length: i64, filler: String) -> Result<String, String> {
    let text = if let Some(i) = value.as_i64() {
        i.to_string()
    } else if let Some(i) = value.as_u64() {
        i.to_string()
    } else if let Some(x) = value.as_f64() {
        fixed(x, 6)
    } else if let Some(b) = value.as_bool() {
        i32::from(b).to_string()
    } else if let Some(i) = value.get("$int").and_then(Value::as_str) {
        i.to_owned()
    } else if let Some(n) = value.get("$num").and_then(Value::as_str) {
        match n {
            "NaN" => "nan".into(),
            "Infinity" => "inf".into(),
            "-Infinity" => "-inf".into(),
            "-0" => "-0.000000".into(),
            _ => return Err("TypeError: invalid numeric value".into()),
        }
    } else {
        return Err("TypeError: invalid numeric value".into());
    };
    Ok(fill_left(text, length, filler)?.replace(',', "."))
}

pub fn format_date_time(
    date: i64,
    has_time_zone: bool,
    date_separator: String,
    time_separator: String,
    separator: String,
    offset_seconds: i64,
) -> Result<String, String> {
    if i32::try_from(offset_seconds.div_euclid(86_400)).is_err() {
        return Err("OverflowError: Python int too large to convert to C int".into());
    }
    if !(-86_399..=86_399).contains(&offset_seconds) {
        return Err(format!(
            "ValueError: offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not {}.",
            offset_description(offset_seconds)
        ));
    }
    if date == 0 {
        return Ok(String::new());
    }
    if cfg!(windows) && !(-43_200..32_536_850_400).contains(&date) {
        return Err("OSError: [Errno 22] Invalid argument".into());
    }
    let value = date
        .checked_add(offset_seconds)
        .and_then(|seconds| Timestamp::from_second(seconds).ok())
        .ok_or_else(|| "OverflowError: date value out of range".to_owned())?
        .to_zoned_datetime(TimeZone::UTC);
    let (day, month, year) = (value.day(), value.month(), value.year());
    if year < 1 {
        return Err("OverflowError: date value out of range".into());
    }
    let (hour, minute, second) = (value.hour(), value.minute(), value.second());
    let mut result = format!(
        "{day:02}{date_separator}{month:02}{date_separator}{year}{separator}{hour:02}{time_separator}{minute:02}{time_separator}{second:02}"
    );
    if has_time_zone {
        let minutes = offset_seconds / 60;
        let sign = if minutes < 0 { '-' } else { '+' };
        let minutes = minutes.abs();
        let (hours, minutes) = (minutes / 60, minutes % 60);
        result.push_str(&format!("{separator}UTC{sign}{hours:02}:{minutes:02}"));
    }
    Ok(result)
}

fn offset_description(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    if rest == 0 {
        format!("datetime.timedelta(days={days})")
    } else {
        format!("datetime.timedelta(days={days}, seconds={rest})")
    }
}

pub fn format_file_size(size: i64) -> Result<String, String> {
    let (divisor, unit) = match size {
        1_048_576.. => (1_048_576, "MB"),
        1024.. => (1024, "KB"),
        _ => return Ok(format!("{size} B")),
    };
    let tenths = i128::from(size) * 10 / divisor;
    Ok(format!("{}.{} {unit}", tenths / 10, tenths % 10))
}

pub fn format_duration(seconds: i64) -> Result<String, String> {
    let (hours, minutes, seconds) = (
        seconds.div_euclid(3600),
        seconds.rem_euclid(3600) / 60,
        seconds.rem_euclid(60),
    );
    Ok(if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    })
}

pub fn format_image_size_text(width: i64, height: i64) -> Result<String, String> {
    Ok(format!("{width}×{height}"))
}

#[derive(Clone, Copy)]
struct CurrencyRule {
    international: &'static str,
    thousands: &'static str,
    decimal: &'static str,
    left: bool,
    space: bool,
    exponent: u32,
}

fn currency_rule(currency: &str) -> CurrencyRule {
    let (international, thousands, decimal, left, space, exponent) = match currency {
        "AED" | "BDT" | "DZD" | "EGP" | "HNL" | "LBP" | "LKR" | "MAD" | "NIO" | "PAB" | "PEN"
        | "QAR" | "SAR" | "SYP" | "YER" | "ZAR" => ("", ",", ".", true, true, 2),
        "ALL" => ("", ".", ",", false, false, 2),
        "AMD" | "MDL" | "MVR" => ("", ",", ".", false, true, 2),
        "ARS" | "BOB" | "COP" | "UYU" | "VEF" => ("", ".", ",", true, true, 2),
        "AUD" => ("AU$", ",", ".", true, false, 2),
        "AZN" | "BGN" | "BYN" | "CZK" | "GEL" | "HUF" | "PLN" | "RUB" | "UZS" => {
            ("", " ", ",", false, true, 2)
        }
        "BAM" | "HRK" | "RON" | "RSD" | "SEK" | "TRY" => ("", ".", ",", false, true, 2),
        "BHD" | "IQD" => ("", ",", ".", true, true, 3),
        "BND" | "CRC" | "IDR" => ("", ".", ",", true, false, 2),
        "BRL" => ("R$", ".", ",", true, true, 2),
        "CAD" => ("CA$", ",", ".", true, false, 2),
        "CHF" => ("", "'", ".", false, true, 2),
        "CLP" | "PYG" => ("", ".", ",", true, true, 0),
        "CNY" => ("CN¥", ",", ".", true, false, 2),
        "DKK" => ("", "\0", ",", false, true, 2),
        "EUR" => ("€", " ", ",", false, true, 2),
        "GBP" => ("£", ",", ".", true, false, 2),
        "HKD" => ("HK$", ",", ".", true, false, 2),
        "ILS" => ("₪", ",", ".", true, true, 2),
        "INR" => ("₹", ",", ".", true, false, 2),
        "IRR" => ("", ",", "/", false, true, 2),
        "ISK" => ("", ".", ",", false, true, 0),
        "JOD" => ("", ",", ".", true, false, 3),
        "JPY" => ("¥", ",", ".", true, false, 0),
        "KGS" => ("", " ", "-", false, true, 2),
        "KRW" => ("₩", ",", ".", true, false, 0),
        "KZT" => ("", " ", "-", true, false, 2),
        "MNT" => ("", " ", ",", true, false, 2),
        "MXN" => ("MX$", ",", ".", true, false, 2),
        "NOK" => ("", " ", ",", true, true, 2),
        "NZD" => ("NZ$", ",", ".", true, false, 2),
        "THB" => ("฿", ",", ".", true, false, 2),
        "TJS" => ("", " ", ";", false, true, 2),
        "TWD" => ("NT$", ",", ".", true, false, 2),
        "UAH" => ("", " ", ",", false, false, 2),
        "UGX" => ("", ",", ".", true, false, 0),
        "USD" => ("$", ",", ".", true, false, 2),
        "VND" => ("₫", ".", ",", false, true, 0),
        _ => ("", ",", ".", true, false, 2),
    };
    CurrencyRule {
        international,
        thousands,
        decimal,
        left,
        space,
        exponent,
    }
}

pub fn lookup_currency_rule(currency: String) -> Result<Value, String> {
    let r = currency_rule(&currency);
    Ok(
        json!({"international":r.international,"thousands":r.thousands,"decimal":r.decimal,"left":r.left,"space":r.space,"exponent":r.exponent,"strip_dot_zero":false}),
    )
}

fn grouped(whole: &str, thousands: &str) -> String {
    let mut out = String::new();
    let digits = if let Some(rest) = whole.strip_prefix('-') {
        out.push('-');
        rest
    } else {
        whole
    };
    if thousands == "\0" || !digits.bytes().all(|b| b.is_ascii_digit()) {
        out.push_str(digits);
        return out;
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(thousands);
        }
        out.push(ch);
    }
    out
}

pub fn format_with_separators(
    amount: f64,
    precision: i64,
    decimal: String,
    thousands: String,
) -> Result<String, String> {
    let precision = usize::try_from(precision)
        .map_err(|_| "ValueError: Format specifier missing precision".to_owned())?;
    if precision > i32::MAX as usize {
        return Err("ValueError: precision too big".into());
    }
    if amount.is_finite() && precision > 100_000 {
        return Err("MemoryError: ".into());
    }
    let exact = fixed(amount, precision);
    let (whole, fraction) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut text = grouped(whole, &thousands);
    if !fraction.is_empty() {
        text.push_str(&decimal);
        text.push_str(fraction);
    }
    Ok(text)
}

pub fn fill_amount_and_currency(
    amount: i64,
    currency: String,
    force_strip_dot_zero: bool,
) -> Result<String, String> {
    let magnitude = amount.unsigned_abs();
    if currency == "XTR" {
        return Ok(format!("⭐{}", grouped(&magnitude.to_string(), ",")));
    }
    let r = currency_rule(&currency);
    let value = magnitude as f64 / f64::from(10_u32.pow(r.exponent));
    let exponent = if force_strip_dot_zero && value.fract() == 0.0 {
        0
    } else {
        r.exponent
    };
    let formatted = format_with_separators(
        value,
        i64::from(exponent),
        r.decimal.into(),
        r.thousands.into(),
    )?;
    let sign = if amount < 0 { "−" } else { "" };
    let spacing = if r.space { " " } else { "" };
    let name = if r.international.is_empty() {
        currency.as_str()
    } else {
        r.international
    };
    Ok(if r.left {
        format!("{sign}{name}{spacing}{formatted}")
    } else {
        format!("{sign}{formatted}{spacing}{name}")
    })
}

pub fn format_money_amount(amount: i64, currency: String) -> Result<String, String> {
    fill_amount_and_currency(amount, currency, false)
}

fn attr<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    value.get(name).ok_or_else(|| {
        format!("AttributeError: 'types.SimpleNamespace' object has no attribute '{name}'")
    })
}

pub fn countries_from_tl(countries: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for country in countries {
        if attr(&country, "hidden")?.as_bool() == Some(true) {
            continue;
        }
        let name = attr(&country, "default_name")?;
        let iso2 = attr(&country, "iso2")?;
        let codes = attr(&country, "country_codes")?
            .as_array()
            .ok_or("TypeError: 'NoneType' object is not iterable")?;
        let mut translated = Vec::with_capacity(codes.len());
        for code in codes {
            let number = attr(code, "country_code")?;
            let prefixes = attr(code, "prefixes")?
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![json!("")]);
            let patterns = attr(code, "patterns")?
                .as_array()
                .cloned()
                .unwrap_or_default();
            translated.push(json!({"calling_code":number,"prefixes":prefixes,"patterns":patterns}));
        }
        result.push(json!({"name":name,"iso2":iso2,"codes":translated,"is_hidden":false}));
    }
    Ok(result)
}

fn formatted_phone(countries: &[Value], phone: &str) -> Result<String, String> {
    if phone.is_empty() || phone.starts_with('0') {
        return Ok(phone.to_owned());
    }
    let collapsed = phone.replace(' ', "");
    let phone = collapsed.strip_prefix('+').unwrap_or(&collapsed);
    let mut best = None;
    let mut length = 0;
    for country in countries {
        for required in ["name", "iso2"] {
            if country.get(required).is_none() {
                if !country.is_object() {
                    return Err("TypeError: 'int' object is not iterable".into());
                }
                return Err("TypeError: CountryInfo.__init__() missing 2 required positional arguments: 'name' and 'iso2'".into());
            }
        }
        for code in country
            .get("codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let number = code.get("calling_code").and_then(Value::as_str)
                .ok_or("TypeError: CallingCodeInfo.__init__() missing 1 required positional argument: 'calling_code'")?;
            let Some(rest) = phone.strip_prefix(number) else {
                continue;
            };
            for prefix in code
                .get("prefixes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let size = number.chars().count() + prefix.chars().count();
                if size > length && rest.starts_with(prefix) {
                    (length, best) = (size, Some((code, number, rest)));
                }
            }
        }
    }
    let Some((code, number, rest)) = best else {
        return Ok(phone.into());
    };
    let mut formatted = rest.to_owned();
    let mut max_matched = 0;
    for pattern in code
        .get("patterns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let mut cursor = pattern.chars().peekable();
        let mut result = String::new();
        let mut matched = 0;
        let mut failed = false;
        for ch in rest.chars() {
            while cursor.peek().is_some_and(|p| *p != 'X' && !p.is_numeric()) {
                if let Some(literal) = cursor.next() {
                    result.push(literal);
                }
            }
            match cursor.next() {
                None | Some('X') => (),
                Some(p) if p == ch => matched += 1,
                Some(_) => {
                    failed = true;
                    break;
                }
            }
            result.push(ch);
        }
        if !failed && matched >= max_matched {
            (max_matched, formatted) = (matched, result);
        }
    }
    Ok(format!("+{number} {formatted}"))
}

pub fn format_phone_sequence(
    country_lists: Vec<Vec<Value>>,
    phones: Vec<String>,
) -> Result<Vec<String>, String> {
    let mismatch = match country_lists.len().cmp(&phones.len()) {
        Ordering::Greater => Some("shorter"),
        Ordering::Less => Some("longer"),
        Ordering::Equal => None,
    };
    let result = country_lists
        .iter()
        .zip(&phones)
        .map(|(countries, phone)| formatted_phone(countries, phone))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(kind) = mismatch {
        return Err(format!(
            "ValueError: zip() argument 2 is {kind} than argument 1"
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_sign_and_grouping() {
        assert_eq!(
            fill_amount_and_currency(-500, "JPY".into(), false),
            Ok("−¥500".into())
        );
        assert_eq!(
            fill_amount_and_currency(100_000, "DKK".into(), false),
            Ok("1000,00 DKK".into())
        );
        assert_eq!(
            fill_amount_and_currency(i64::MIN, "XTR".into(), false),
            Ok("⭐9,223,372,036,854,775,808".into())
        );
    }

    #[test]
    fn rejects_negative_precision() {
        assert_eq!(
            format_with_separators(1.2, -1, ".".into(), ",".into()),
            Err("ValueError: Format specifier missing precision".into())
        );
    }

    #[test]
    fn treats_sizes_and_durations_at_boundaries() {
        assert_eq!(format_file_size(1023), Ok("1023 B".into()));
        assert_eq!(format_file_size(1024), Ok("1.0 KB".into()));
        assert_eq!(format_duration(-1), Ok("-1:59:59".into()));
        assert_eq!(format_duration(3600), Ok("1:00:00".into()));
    }

    #[test]
    fn keeps_phone_sequence_state_local() {
        let ru = json!({"name":"Synthetic","iso2":"ZZ","codes":[{"calling_code":"7","prefixes":[""],"patterns":["XXX XXX XXXX"]}]});
        assert_eq!(
            format_phone_sequence(
                vec![vec![ru], vec![]],
                vec!["+7 9991234567".into(), "79991234567".into()]
            ),
            Ok(vec!["+7 999 123 4567".into(), "79991234567".into()])
        );
    }
}
