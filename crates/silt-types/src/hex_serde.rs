/// Serde for a 128-bit id newtype: 32 hex digits in human-readable formats
/// such as JSON, where a 128-bit number would lose precision in JavaScript
/// and other clients; the bare number in binary ones such as postcard, so
/// what is stored stays as it was.
macro_rules! hex_serde {
    ($name:ident) => {
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                if serializer.is_human_readable() { serializer.collect_str(self) } else { self.0.serialize(serializer) }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                if deserializer.is_human_readable() {
                    let text = String::deserialize(deserializer)?;
                    text.parse().map_err(serde::de::Error::custom)
                } else {
                    u128::deserialize(deserializer).map(Self)
                }
            }
        }
    };
}

pub(crate) use hex_serde;
