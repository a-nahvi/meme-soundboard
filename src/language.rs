use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    Persian,
}

impl Language {
    pub fn text<T>(self, english: T, persian: T) -> T {
        match self {
            Self::English => english,
            Self::Persian => persian,
        }
    }

    pub fn is_rtl(self) -> bool {
        self == Self::Persian
    }
}
