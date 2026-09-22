use super::{Error, der::Element};
pub(super) fn parse(value: Element<'_>) -> Result<i64, Error> {
    let bytes = value.body;
    let digits = match value.tag {
        0x17 => 12,
        0x18 => 14,
        _ => return Err(Error::Encoding),
    };
    if bytes.len() != digits + 1
        || bytes[digits] != b'Z'
        || !bytes[..digits].iter().all(u8::is_ascii_digit)
    {
        return Err(Error::Encoding);
    }
    let number = |start: usize, count: usize| {
        bytes[start..start + count]
            .iter()
            .fold(0i64, |n, b| n * 10 + i64::from(b - b'0'))
    };
    let (year, offset) = if digits == 12 {
        let y = number(0, 2);
        (if y >= 50 { 1900 + y } else { 2000 + y }, 2)
    } else {
        let y = number(0, 4);
        if y < 2050 {
            return Err(Error::Encoding);
        }
        (y, 4)
    };
    let month = number(offset, 2);
    let day = number(offset + 2, 2);
    let hour = number(offset + 4, 2);
    let minute = number(offset + 6, 2);
    let second = number(offset + 8, 2);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
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
    if !(1..=12).contains(&month)
        || day < 1
        || day > months[month as usize - 1]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(Error::Encoding);
    }
    let leaps = |y: i64| y / 4 - y / 100 + y / 400;
    let days = (year - 1970) * 365 + leaps(year - 1) - leaps(1969)
        + months[..month as usize - 1].iter().sum::<i64>()
        + day
        - 1;
    Ok(days * 86400 + hour * 3600 + minute * 60 + second)
}
