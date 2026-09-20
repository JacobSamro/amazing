//! RFC6265bis-22 section 5.1.1 token matching; no locale/date library.
fn digits(token: &[u8], min: usize, max: usize) -> Option<(i64, usize)> {
    let n = token.iter().take_while(|b| b.is_ascii_digit()).count();
    if n < min || n > max {
        return None;
    }
    Some((
        token[..n].iter().fold(0, |v, b| v * 10 + (b - b'0') as i64),
        n,
    ))
}
fn time(token: &[u8]) -> Option<(i64, i64, i64)> {
    let (h, a) = digits(token, 1, 2)?;
    if token.get(a) != Some(&b':') {
        return None;
    }
    let token = &token[a + 1..];
    let (m, b) = digits(token, 1, 2)?;
    if token.get(b) != Some(&b':') {
        return None;
    }
    let (s, _) = digits(&token[b + 1..], 1, 2)?;
    Some((h, m, s))
}
/// UTC seconds relative to 1970, including representable pre-epoch dates.
pub(super) fn parse_date(input: &[u8]) -> Option<i64> {
    let (mut day, mut month, mut year, mut clock) = (None, None, None, None);
    for token in input
        .split(|b| matches!(*b,9|0x20..=0x2f|0x3b..=0x40|0x5b..=0x60|0x7b..=0x7e))
        .filter(|v| !v.is_empty())
    {
        if clock.is_none() {
            if let Some(v) = time(token) {
                clock = Some(v);
                continue;
            }
        }
        if day.is_none() {
            if let Some((v, _)) = digits(token, 1, 2) {
                day = Some(v);
                continue;
            }
        }
        if month.is_none() && token.len() >= 3 {
            if let Some(m) = [
                b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct",
                b"nov", b"dec",
            ]
            .iter()
            .position(|m| token[..3].eq_ignore_ascii_case(*m))
            {
                month = Some(m + 1);
                continue;
            }
        }
        if year.is_none() {
            if let Some((v, _)) = digits(token, 2, 4) {
                year = Some(v);
            }
        }
    }
    let (d, m, mut y, (h, min, s)) = (day?, month?, year?, clock?);
    if y <= 69 {
        y += 2000;
    } else if y <= 99 {
        y += 1900;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if y < 1601 || d < 1 || d > days[m - 1] || h > 23 || min > 59 || s > 59 {
        return None;
    }
    // Gregorian days before year, then before month. All inputs <=9999.
    fn before_year(y: i64) -> i64 {
        let n = y - 1;
        365 * n + n / 4 - n / 100 + n / 400
    }
    let date = before_year(y) - before_year(1970) + days[..m - 1].iter().sum::<i64>() + d - 1;
    Some(date * 86400 + h * 3600 + min * 60 + s)
}
