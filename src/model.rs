use serde::{Deserialize, Deserializer, Serialize};

/// An event sent by a Phrasea application (`alchemy/report-sdk`).
///
/// Absent or null text fields become empty strings, an absent `eventDate`
/// becomes 0 and unknown fields are ignored: the rows are written as they come,
/// the columns decide what is accepted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogEntry {
    #[serde(deserialize_with = "null_as_default")]
    pub app_name: String,
    #[serde(deserialize_with = "null_as_default")]
    pub app_id: String,
    #[serde(deserialize_with = "null_as_default")]
    pub action: String,
    #[serde(deserialize_with = "null_as_default")]
    pub item: String,
    #[serde(deserialize_with = "null_as_default")]
    pub user: String,
    /// Unix timestamp, in seconds.
    #[serde(deserialize_with = "null_as_default")]
    pub event_date: i64,
    pub payload: Option<serde_json::Value>,
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_a_full_entry() {
        let entry: LogEntry = serde_json::from_value(json!({
            "appName": "databox",
            "appId": "6b0e4c1e-0d0b-4bd4-a1f3-9a3c1b0e5f2d",
            "action": "asset_view",
            "item": "asset-id",
            "user": "user-id",
            "eventDate": 1750000000,
            "payload": {"a": [1, 2]},
        }))
        .unwrap();

        assert_eq!(
            entry,
            LogEntry {
                app_name: "databox".into(),
                app_id: "6b0e4c1e-0d0b-4bd4-a1f3-9a3c1b0e5f2d".into(),
                action: "asset_view".into(),
                item: "asset-id".into(),
                user: "user-id".into(),
                event_date: 1750000000,
                payload: Some(json!({"a": [1, 2]})),
            }
        );
    }

    #[test]
    fn missing_and_null_fields_take_their_default() {
        let entry: LogEntry =
            serde_json::from_value(json!({"action": "test", "item": null, "eventDate": null}))
                .unwrap();

        assert_eq!(
            entry,
            LogEntry {
                action: "test".into(),
                ..LogEntry::default()
            }
        );
    }

    #[test]
    fn ignores_unknown_fields_and_accepts_any_payload() {
        let entry: LogEntry =
            serde_json::from_value(json!({"action": "test", "other": 1, "payload": [1, "x"]}))
                .unwrap();

        assert_eq!(entry.payload, Some(json!([1, "x"])));
    }

    #[test]
    fn rejects_a_mistyped_field() {
        assert!(serde_json::from_value::<LogEntry>(json!({"eventDate": "yesterday"})).is_err());
        assert!(serde_json::from_value::<LogEntry>(json!({"eventDate": 1.5})).is_err());
        assert!(serde_json::from_value::<LogEntry>(json!({"action": 3})).is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let entry = LogEntry {
            action: "test".into(),
            event_date: 42,
            payload: Some(json!({"k": "v"})),
            ..LogEntry::default()
        };

        let line = serde_json::to_string(&entry).unwrap();

        assert_eq!(serde_json::from_str::<LogEntry>(&line).unwrap(), entry);
    }
}
