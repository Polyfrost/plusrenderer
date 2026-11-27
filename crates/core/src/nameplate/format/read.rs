use serde_json::Value;

use crate::issues::{join, path, Issue, IssueCode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Loc {
    pub file: String,
    pub at: String,
}

impl Loc {
    pub fn new(file: &str, at: &str) -> Loc {
        Loc {
            file: file.to_string(),
            at: at.to_string(),
        }
    }

    pub fn key(&self, key: &str) -> Loc {
        Loc {
            file: self.file.clone(),
            at: join(&self.at, key),
        }
    }

    pub fn index(&self, i: usize) -> Loc {
        Loc {
            file: self.file.clone(),
            at: format!("{}[{i}]", self.at),
        }
    }

    pub fn path(&self) -> String {
        path(&self.file, &self.at)
    }

    pub fn error(&self, code: IssueCode) -> Issue {
        Issue::error(code, self.path())
    }
}

pub(super) fn whole(value: &Value) -> Option<i64> {
    value.as_i64()
}

pub(super) fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

pub(super) fn hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    if !(digits.len() == 6 || digits.len() == 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(if digits.len() == 6 {
        value | 0xff00_0000
    } else {
        value
    })
}

pub(super) fn slot_ref(text: &str) -> Option<(&str, u8)> {
    let (id, tone) = match text.split_once('[') {
        None => (text, 0),
        Some((id, rest)) => {
            let digits = rest.strip_suffix(']')?;
            if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            (id, digits.parse::<u8>().ok()?)
        }
    };
    crate::container::is_id(id).then_some((id, tone))
}

pub(super) fn pair_i64(value: &Value) -> Option<[i64; 2]> {
    let items = value.as_array().filter(|a| a.len() == 2)?;
    Some([whole(&items[0])?, whole(&items[1])?])
}

pub(super) fn pair_f64(value: &Value) -> Option<[f64; 2]> {
    let items = value.as_array().filter(|a| a.len() == 2)?;
    Some([number(&items[0])?, number(&items[1])?])
}

pub(super) fn fixed_point(value: f64, unit: f64) -> i32 {
    libm::floor(value * unit + 0.5) as i32
}
