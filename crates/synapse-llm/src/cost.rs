//! Cost tracking: usage × catalog price per message, totals per conversation.
//!
//! Each assistant turn's [`Usage`] is priced with the models.dev catalog
//! ([`Catalog::estimate_cost`]) and stored as a [`MessageCost`] (persist the
//! `meta_json` into the message `meta` column). [`ConversationCost`]
//! aggregates one conversation for the header/title bar
//! ([`ConversationCost::header_label`]).

use serde::{Deserialize, Serialize};

use crate::backend::Usage;
use crate::catalog::{Catalog, CatalogError};

/// Priced cost of a single assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageCost {
    pub message_id: String,
    pub conversation_id: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// USD for this message (input + output legs at catalog rates).
    pub cost_usd: f64,
}

impl MessageCost {
    /// Quiet per-message footer, e.g. `~$0.0012 · 1.2k tokens`. The cost is
    /// left out when the model has no catalog price (local, custom endpoints).
    pub fn footer_label(&self) -> String {
        let tokens = format_tokens(self.input_tokens + self.output_tokens);
        if self.cost_usd > 0.0 {
            format!("~{} · {tokens}", format_usd(self.cost_usd))
        } else {
            tokens
        }
    }

    /// Price one message with the catalog. Unknown models price at 0.0
    /// (local models, custom endpoints) rather than failing the turn.
    pub fn new(
        catalog: &Catalog,
        message_id: impl Into<String>,
        conversation_id: impl Into<String>,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        usage: Usage,
    ) -> Self {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        let cost_usd = catalog
            .estimate_cost(
                &provider_id,
                &model_id,
                usage.input_tokens,
                usage.output_tokens,
            )
            .unwrap_or(0.0);
        Self {
            message_id: message_id.into(),
            conversation_id: conversation_id.into(),
            provider_id,
            model_id,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: sanitize(cost_usd),
        }
    }

    /// Strict variant that surfaces [`CatalogError::UnknownModel`] etc.
    pub fn new_strict(
        catalog: &Catalog,
        message_id: impl Into<String>,
        conversation_id: impl Into<String>,
        provider_id: &str,
        model_id: &str,
        usage: Usage,
    ) -> Result<Self, CatalogError> {
        let cost_usd = catalog.estimate_cost(
            provider_id,
            model_id,
            usage.input_tokens,
            usage.output_tokens,
        )?;
        Ok(Self {
            message_id: message_id.into(),
            conversation_id: conversation_id.into(),
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: sanitize(cost_usd),
        })
    }

    /// Encode for the `messages.meta` JSON column (`{"cost": {...}}` merge).
    pub fn meta_json(&self) -> String {
        serde_json::json!({
            "provider": self.provider_id,
            "model": self.model_id,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cost_usd": self.cost_usd,
        })
        .to_string()
    }

    /// Decode a cost previously stored with [`MessageCost::meta_json`].
    pub fn from_meta(
        message_id: impl Into<String>,
        conversation_id: impl Into<String>,
        meta: &str,
    ) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(meta).ok()?;
        let c = v.get("cost").unwrap_or(&v);
        Some(Self {
            message_id: message_id.into(),
            conversation_id: conversation_id.into(),
            provider_id: c
                .get("provider")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            model_id: c
                .get("model")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            input_tokens: c.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            output_tokens: c.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            cost_usd: c.get("cost_usd").and_then(|x| x.as_f64()).unwrap_or(0.0),
        })
    }
}

fn sanitize(v: f64) -> f64 {
    if v.is_finite() && v >= 0.0 {
        v
    } else {
        0.0
    }
}

/// Totals for one conversation, shown in the header/title bar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationCost {
    pub conversation_id: String,
    pub messages: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_usd: f64,
}

impl ConversationCost {
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            ..Self::default()
        }
    }

    pub fn add(&mut self, m: &MessageCost) {
        self.messages += 1;
        self.input_tokens = self.input_tokens.saturating_add(m.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(m.output_tokens);
        self.total_usd = sanitize(self.total_usd + m.cost_usd);
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Aggregate a conversation's message costs.
    pub fn summarize(conversation_id: impl Into<String>, costs: &[MessageCost]) -> Self {
        let mut out = Self::new(conversation_id);
        for c in costs {
            out.add(c);
        }
        out
    }

    /// Short label for the chat header / title bar, e.g. `$0.0123 · 4 msgs`.
    /// Empty conversation renders as `$0.00`.
    pub fn header_label(&self) -> String {
        if self.messages == 0 {
            return format!("{} · 0 msgs", format_usd(0.0));
        }
        let msg_word = if self.messages == 1 { "msg" } else { "msgs" };
        format!(
            "{} · {} {msg_word} · {}",
            format_usd(self.total_usd),
            self.messages,
            format_tokens(self.total_tokens())
        )
    }

    /// Longer title-bar tooltip line with the token split.
    pub fn title_text(&self, title: &str) -> String {
        format!(
            "{title} — {} ({} in / {} out tokens)",
            format_usd(self.total_usd),
            self.input_tokens,
            self.output_tokens
        )
    }
}

/// USD formatting: sub-cent values keep 4 decimals, else 2.
pub fn format_usd(v: f64) -> String {
    let v = sanitize(v);
    if v < 0.01 && v > 0.0 {
        format!("${v:.4}")
    } else {
        format!("${v:.2}")
    }
}

/// Compact token formatting: 1500 -> "1.5k".
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M tokens", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k tokens", n as f64 / 1000.0)
    } else {
        format!("{n} tokens")
    }
}

#[cfg(test)]
mod cost_tests {
    use super::*;

    fn demo_catalog() -> Catalog {
        let raw = serde_json::json!({
            "demo": {
                "id": "demo", "name": "Demo", "env": [],
                "models": {
                    "m1": {
                        "id": "m1", "name": "M1",
                        "cost": {"input": 1.0, "output": 2.0}
                    },
                    "free": {
                        "id": "free", "name": "Free",
                        "cost": {"input": 0.0, "output": 0.0}
                    }
                }
            }
        });
        Catalog::from_json(&serde_json::to_vec(&raw).unwrap()).unwrap()
    }

    #[test]
    fn cost_message_prices_usage_times_catalog_rate() {
        let cat = demo_catalog();
        let m = MessageCost::new(
            &cat,
            "msg1",
            "conv1",
            "demo",
            "m1",
            Usage::new(1_000_000, 500_000),
        );
        assert_eq!(m.input_tokens, 1_000_000);
        assert_eq!(m.output_tokens, 500_000);
        assert!((m.cost_usd - 2.0).abs() < 1e-9, "got {}", m.cost_usd);
    }

    #[test]
    fn cost_unknown_model_prices_zero_and_strict_errors() {
        let cat = demo_catalog();
        let m = MessageCost::new(&cat, "msg1", "conv1", "demo", "nope", Usage::new(100, 100));
        assert_eq!(m.cost_usd, 0.0);
        assert!(MessageCost::new_strict(&cat, "m", "c", "demo", "nope", Usage::new(1, 1)).is_err());
    }

    #[test]
    fn cost_meta_roundtrip_for_store() {
        let cat = demo_catalog();
        let m = MessageCost::new(&cat, "msg1", "conv1", "demo", "m1", Usage::new(10, 5));
        let meta = format!("{{\"cost\": {}}}", m.meta_json());
        let back = MessageCost::from_meta("msg1", "conv1", &meta).unwrap();
        assert_eq!(back, m);
        assert!(MessageCost::from_meta("m", "c", "not json").is_none());
    }

    #[test]
    fn cost_conversation_totals_and_header_label() {
        let cat = demo_catalog();
        let a = MessageCost::new(&cat, "a", "c1", "demo", "m1", Usage::new(1_000_000, 0));
        let b = MessageCost::new(&cat, "b", "c1", "demo", "m1", Usage::new(0, 1_000_000));
        let total = ConversationCost::summarize("c1", &[a, b]);
        assert_eq!(total.messages, 2);
        assert_eq!(total.input_tokens, 1_000_000);
        assert_eq!(total.output_tokens, 1_000_000);
        assert!((total.total_usd - 3.0).abs() < 1e-9);
        let label = total.header_label();
        assert!(label.contains("$3.00"), "{label}");
        assert!(label.contains("2 msgs"), "{label}");
        assert!(label.contains("tokens"), "{label}");
        assert!(total.title_text("Chat").starts_with("Chat — "));
        let empty = ConversationCost::new("c2");
        assert!(empty.header_label().contains("0 msgs"));
        assert_eq!(format_usd(0.00123), "$0.0012");
        assert_eq!(format_usd(1.5), "$1.50");
        assert_eq!(format_tokens(999), "999 tokens");
        assert_eq!(format_tokens(1500), "1.5k tokens");
    }
}
