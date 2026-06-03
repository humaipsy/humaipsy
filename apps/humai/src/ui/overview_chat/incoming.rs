// Ported from openchat/src/incoming/source.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageSource {
    Human,
    Api,
    #[default]
    System,
}

impl MessageSource {
    pub fn as_db(self) -> &'static str {
        // Stable lowercase labels preserve compatibility across app versions.
        match self {
            MessageSource::Human => "human",
            MessageSource::Api => "api",
            MessageSource::System => "system",
        }
    }

    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "human" => Some(Self::Human),
            "api" => Some(Self::Api),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}
