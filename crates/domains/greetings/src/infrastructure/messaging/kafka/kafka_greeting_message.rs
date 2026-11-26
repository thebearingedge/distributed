use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::greeting::GreetingModel;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct KafkaGreetingMessage {
    pub(super) id: Uuid,
    pub(super) message: String,
}

impl From<&GreetingModel> for KafkaGreetingMessage {
    fn from(greeting: &GreetingModel) -> Self {
        Self {
            id: greeting.id,
            message: greeting.message.clone(),
        }
    }
}

impl From<KafkaGreetingMessage> for GreetingModel {
    fn from(message: KafkaGreetingMessage) -> Self {
        Self {
            id: message.id,
            message: message.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use crate::models::greeting::GreetingModel;

    use super::KafkaGreetingMessage;

    #[test]
    fn wire_format_is_id_and_message() {
        let id = Uuid::now_v7();
        let message = KafkaGreetingMessage {
            id,
            message: "hi".into(),
        };

        let value = serde_json::to_value(&message).expect("wire message serializes");

        assert_eq!(value, json!({ "id": id.to_string(), "message": "hi" }));
    }

    #[test]
    fn round_trips_through_the_model() {
        let greeting = GreetingModel::new("hello");

        let model = GreetingModel::from(KafkaGreetingMessage::from(&greeting));

        assert_eq!(model, greeting);
    }
}
