//! Language codes ("en", "it", "zh-CN"), kept apart from words and the rest
//! of the text they travel with.
//!
//! Any code is taken as it is: they come from the settings, the dictionaries
//! and the translation services, and each place checks what it can use.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

/// A language's code, as the settings, the dictionaries and the translation
/// services name it.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LangCode(String);

impl LangCode {
    pub fn new(code: impl Into<String>) -> LangCode {
        LangCode(code.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for LangCode {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LangCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for LangCode {
    fn from(code: &str) -> LangCode {
        LangCode::new(code)
    }
}

impl From<String> for LangCode {
    fn from(code: String) -> LangCode {
        LangCode(code)
    }
}

impl PartialEq<str> for LangCode {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for LangCode {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_as_a_plain_string() {
        let code: LangCode = serde_json::from_str(r#""zh-CN""#).unwrap();
        assert_eq!(code, "zh-CN");
        assert_eq!(serde_json::to_string(&code).unwrap(), r#""zh-CN""#);
        assert_eq!(code.to_string(), "zh-CN");
    }
}
