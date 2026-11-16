//! Desktop currency table and amount formatting.
pub fn format_grams_amount(nanos: i64) -> String {
    const NANOS: u64 = 1_000_000_000;
    let absolute = nanos.unsigned_abs();
    let fraction = format!("{:09}", absolute % NANOS);
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{}{}{} {}",
        if nanos < 0 { "-" } else { "" },
        absolute / NANOS,
        if fraction.is_empty() {
            String::new()
        } else {
            format!(".{fraction}")
        },
        if absolute == NANOS { "Gram" } else { "Grams" }
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencyRule {
    pub international: &'static str,
    pub thousands: &'static str,
    pub decimal: &'static str,
    pub left: bool,
    pub space: bool,
    pub exponent: u32,
    pub strip_dot_zero: bool,
}
impl Default for CurrencyRule {
    fn default() -> Self {
        Self {
            international: "",
            thousands: ",",
            decimal: ".",
            left: true,
            space: false,
            exponent: 2,
            strip_dot_zero: false,
        }
    }
}
pub fn lookup_currency_rule(currency: &str) -> CurrencyRule {
    CURRENCY_RULES
        .binary_search_by_key(&currency, |row| row.0)
        .map_or_else(
            |_| CurrencyRule::default(),
            |index| {
                let (international, thousands, decimal, left, space, exponent, strip_dot_zero) =
                    CURRENCY_RULES[index].1;
                CurrencyRule {
                    international,
                    thousands,
                    decimal,
                    left,
                    space,
                    exponent,
                    strip_dot_zero,
                }
            },
        )
}
pub fn format_with_separators(
    amount: f64,
    precision: usize,
    decimal: &str,
    thousands: &str,
) -> String {
    let text = format!("{amount:.precision$}");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let chars = whole.chars().collect::<Vec<_>>();
    let mut grouped = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0
            && (chars.len() - i) % 3 == 0
            && *c != '-'
            && chars[i - 1] != '-'
            && thousands != "\0"
        {
            grouped.push_str(thousands);
        }
        grouped.push(*c);
    }
    if !fraction.is_empty() {
        grouped.push_str(decimal);
        grouped.push_str(fraction);
    }
    grouped
}
pub fn fill_amount_and_currency(amount: i64, currency: &str, force_strip_dot_zero: bool) -> String {
    if currency == "XTR" {
        return "⭐".to_owned()
            + &format_with_separators(amount.unsigned_abs() as f64, 0, ".", ",");
    }
    let rule = lookup_currency_rule(currency);
    let value = amount.unsigned_abs() as f64 / 10f64.powi(rule.exponent as i32);
    let name = if rule.international.is_empty() {
        currency
    } else {
        rule.international
    };
    let precision = if (rule.strip_dot_zero || force_strip_dot_zero) && value.floor() == value {
        0
    } else {
        rule.exponent as usize
    };
    let number = format_with_separators(value, precision, rule.decimal, rule.thousands);
    let space = if rule.space { " " } else { "" };
    let prefix = if amount < 0 { "−" } else { "" };
    if rule.left {
        format!("{prefix}{name}{space}{number}")
    } else {
        format!("{prefix}{number}{space}{name}")
    }
}
pub fn format_money_amount(amount: i64, currency: &str) -> String {
    fill_amount_and_currency(amount, currency, false)
}

// international symbol, thousands separator, decimal separator, left, space,
// exponent, strip a zero fraction. These immutable values are Desktop data.
type CurrencyData = (
    &'static str,
    &'static str,
    &'static str,
    bool,
    bool,
    u32,
    bool,
);
const CURRENCY_RULES: &[(&str, CurrencyData)] = &[
    ("AED", ("", ",", ".", true, true, 2, false)),
    ("AFN", ("", ",", ".", true, false, 2, false)),
    ("ALL", ("", ".", ",", false, false, 2, false)),
    ("AMD", ("", ",", ".", false, true, 2, false)),
    ("ARS", ("", ".", ",", true, true, 2, false)),
    ("AUD", ("AU$", ",", ".", true, false, 2, false)),
    ("AZN", ("", " ", ",", false, true, 2, false)),
    ("BAM", ("", ".", ",", false, true, 2, false)),
    ("BDT", ("", ",", ".", true, true, 2, false)),
    ("BGN", ("", " ", ",", false, true, 2, false)),
    ("BHD", ("", ",", ".", true, true, 3, false)),
    ("BND", ("", ".", ",", true, false, 2, false)),
    ("BOB", ("", ".", ",", true, true, 2, false)),
    ("BRL", ("R$", ".", ",", true, true, 2, false)),
    ("BYN", ("", " ", ",", false, true, 2, false)),
    ("CAD", ("CA$", ",", ".", true, false, 2, false)),
    ("CHF", ("", "'", ".", false, true, 2, false)),
    ("CLP", ("", ".", ",", true, true, 0, false)),
    ("CNY", ("CN¥", ",", ".", true, false, 2, false)),
    ("COP", ("", ".", ",", true, true, 2, false)),
    ("CRC", ("", ".", ",", true, false, 2, false)),
    ("CZK", ("", " ", ",", false, true, 2, false)),
    ("DKK", ("", "\0", ",", false, true, 2, false)),
    ("DOP", ("", ",", ".", true, false, 2, false)),
    ("DZD", ("", ",", ".", true, true, 2, false)),
    ("EGP", ("", ",", ".", true, true, 2, false)),
    ("ETB", ("", ",", ".", true, false, 2, false)),
    ("EUR", ("€", " ", ",", false, true, 2, false)),
    ("GBP", ("£", ",", ".", true, false, 2, false)),
    ("GEL", ("", " ", ",", false, true, 2, false)),
    ("GHS", ("", ",", ".", true, false, 2, false)),
    ("GTQ", ("", ",", ".", true, false, 2, false)),
    ("HKD", ("HK$", ",", ".", true, false, 2, false)),
    ("HNL", ("", ",", ".", true, true, 2, false)),
    ("HRK", ("", ".", ",", false, true, 2, false)),
    ("HUF", ("", " ", ",", false, true, 2, false)),
    ("IDR", ("", ".", ",", true, false, 2, false)),
    ("ILS", ("₪", ",", ".", true, true, 2, false)),
    ("INR", ("₹", ",", ".", true, false, 2, false)),
    ("IQD", ("", ",", ".", true, true, 3, false)),
    ("IRR", ("", ",", "/", false, true, 2, false)),
    ("ISK", ("", ".", ",", false, true, 0, false)),
    ("JMD", ("", ",", ".", true, false, 2, false)),
    ("JOD", ("", ",", ".", true, false, 3, false)),
    ("JPY", ("¥", ",", ".", true, false, 0, false)),
    ("KES", ("", ",", ".", true, false, 2, false)),
    ("KGS", ("", " ", "-", false, true, 2, false)),
    ("KRW", ("₩", ",", ".", true, false, 0, false)),
    ("KZT", ("", " ", "-", true, false, 2, false)),
    ("LBP", ("", ",", ".", true, true, 2, false)),
    ("LKR", ("", ",", ".", true, true, 2, false)),
    ("MAD", ("", ",", ".", true, true, 2, false)),
    ("MDL", ("", ",", ".", false, true, 2, false)),
    ("MMK", ("", ",", ".", true, false, 2, false)),
    ("MNT", ("", " ", ",", true, false, 2, false)),
    ("MOP", ("", ",", ".", true, false, 2, false)),
    ("MUR", ("", ",", ".", true, false, 2, false)),
    ("MVR", ("", ",", ".", false, true, 2, false)),
    ("MXN", ("MX$", ",", ".", true, false, 2, false)),
    ("MYR", ("", ",", ".", true, false, 2, false)),
    ("MZN", ("", ",", ".", true, false, 2, false)),
    ("NGN", ("", ",", ".", true, false, 2, false)),
    ("NIO", ("", ",", ".", true, true, 2, false)),
    ("NOK", ("", " ", ",", true, true, 2, false)),
    ("NPR", ("", ",", ".", true, false, 2, false)),
    ("NZD", ("NZ$", ",", ".", true, false, 2, false)),
    ("PAB", ("", ",", ".", true, true, 2, false)),
    ("PEN", ("", ",", ".", true, true, 2, false)),
    ("PHP", ("", ",", ".", true, false, 2, false)),
    ("PKR", ("", ",", ".", true, false, 2, false)),
    ("PLN", ("", " ", ",", false, true, 2, false)),
    ("PYG", ("", ".", ",", true, true, 0, false)),
    ("QAR", ("", ",", ".", true, true, 2, false)),
    ("RON", ("", ".", ",", false, true, 2, false)),
    ("RSD", ("", ".", ",", false, true, 2, false)),
    ("RUB", ("", " ", ",", false, true, 2, false)),
    ("SAR", ("", ",", ".", true, true, 2, false)),
    ("SEK", ("", ".", ",", false, true, 2, false)),
    ("SGD", ("", ",", ".", true, false, 2, false)),
    ("SYP", ("", ",", ".", true, true, 2, false)),
    ("THB", ("฿", ",", ".", true, false, 2, false)),
    ("TJS", ("", " ", ";", false, true, 2, false)),
    ("TRY", ("", ".", ",", false, true, 2, false)),
    ("TTD", ("", ",", ".", true, false, 2, false)),
    ("TWD", ("NT$", ",", ".", true, false, 2, false)),
    ("TZS", ("", ",", ".", true, false, 2, false)),
    ("UAH", ("", " ", ",", false, false, 2, false)),
    ("UGX", ("", ",", ".", true, false, 0, false)),
    ("USD", ("$", ",", ".", true, false, 2, false)),
    ("UYU", ("", ".", ",", true, true, 2, false)),
    ("UZS", ("", " ", ",", false, true, 2, false)),
    ("VEF", ("", ".", ",", true, true, 2, false)),
    ("VND", ("₫", ".", ",", false, true, 0, false)),
    ("YER", ("", ",", ".", true, true, 2, false)),
    ("ZAR", ("", ",", ".", true, true, 2, false)),
];
