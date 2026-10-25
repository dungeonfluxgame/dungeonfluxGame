//! `application/x-www-form-urlencoded` bodies from the launcher's own pages.

/// Decoded name/value pairs in submission order. The first value of a repeated name wins.
#[derive(Debug, Default)]
pub struct Form {
    pairs: Vec<(String, String)>,
}

/// Bodies larger than this are refused before decoding; the largest real form is a few KiB.
pub const MAXIMUM_FORM_BYTES: usize = 64 * 1024;

impl Form {
    pub fn parse(body: &[u8]) -> Option<Self> {
        if body.len() > MAXIMUM_FORM_BYTES {
            return None;
        }
        let text = std::str::from_utf8(body).ok()?;
        let mut pairs = Vec::new();
        for part in text.split('&').filter(|part| !part.is_empty()) {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            pairs.push((decode(name)?, decode(value)?));
        }
        Some(Self { pairs })
    }

    #[cfg(test)]
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            pairs: pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded_bytes = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'+' => decoded_bytes.push(b' '),
            b'%' => {
                let high = hex(*bytes.get(index + 1)?)?;
                let low = hex(*bytes.get(index + 2)?)?;
                decoded_bytes.push(high << 4 | low);
                index += 2;
            }
            other => decoded_bytes.push(other),
        }
        index += 1;
    }
    String::from_utf8(decoded_bytes).ok()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plus_percent_and_unicode() {
        let form =
            Form::parse(b"name=The+Gilded%20Tide&key=sk%2Dx%3D1&rune=%E6%98%9F&empty=").unwrap();
        assert_eq!(form.get("name"), Some("The Gilded Tide"));
        assert_eq!(form.get("key"), Some("sk-x=1"));
        assert_eq!(form.get("rune"), Some("星"));
        assert_eq!(form.get("empty"), Some(""));
        assert_eq!(form.get("absent"), None);
    }

    #[test]
    fn refuses_broken_escapes_invalid_utf8_and_oversized_bodies() {
        assert!(Form::parse(b"a=%G1").is_none());
        assert!(Form::parse(b"a=%4").is_none());
        assert!(Form::parse(b"a=%FF").is_none());
        assert!(Form::parse(&vec![b'a'; MAXIMUM_FORM_BYTES + 1]).is_none());
    }

    #[test]
    fn first_value_of_a_repeated_name_wins() {
        let form = Form::parse(b"a=1&a=2").unwrap();
        assert_eq!(form.get("a"), Some("1"));
    }
}
