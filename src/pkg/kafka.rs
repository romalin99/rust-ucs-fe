/// Kafka integration — config structs, topic constants, and client stubs.
///
/// Full port of Go's `pkg/kafka/` package:
///   - `config.go`   → [`ProducerConfig`], [`ConsumerConfig`], [`Config`] (with merge helpers)
///   - `topic.go`    → Topic name constants and message payload types
///   - `producer.go` → [`Producer`] (stub — requires a Kafka client crate in production)
///   - `consumer.go` → [`Consumer`] (stub — requires a Kafka client crate in production)
///
/// ## Production wiring
///
/// The Go implementation uses `franz-go` (`kgo`).  The equivalent Rust crate
/// would be `rdkafka` or `kafka` (not currently in `Cargo.toml`).
/// When a Kafka crate is available, replace the stub bodies below with real
/// broker connections following the same patterns as the Go implementation.
use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

// ─────────────────────────────────────────────────────────────────────────────
// config.go
// ─────────────────────────────────────────────────────────────────────────────

/// Per-topic producer overrides.
///
/// Zero values are ignored; global [`ProducerConfig`] defaults apply instead.
/// Mirrors Go's `ProducerTopicConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProducerTopicConfig {
    #[serde(default)]
    pub acks: String,
    #[serde(default)]
    pub compression: String,
    #[serde(default)]
    pub retries: i32,
    #[serde(default)]
    pub retry_backoff_ms: i32,
    #[serde(default)]
    pub linger_ms: i32,
    #[serde(default)]
    pub batch_size: i32,
    #[serde(default)]
    pub max_message_bytes: i32,
}

/// Global producer configuration with optional per-topic overrides.
///
/// Mirrors Go's `ProducerConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProducerConfig {
    #[serde(default)]
    pub brokers: Vec<String>,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub compression: String,
    #[serde(default)]
    pub linger_ms: i32,
    #[serde(default)]
    pub required_acks: i32,
    #[serde(default)]
    pub request_timeout_ms: i32,
    #[serde(default)]
    pub batch_max_bytes: i32,
    #[serde(default)]
    pub topics: HashMap<String, ProducerTopicConfig>,
}

impl ProducerConfig {
    /// Merge global defaults with per-topic overrides.
    ///
    /// Mirrors Go's `ProducerConfig.EffectiveTopicConfig`.
    pub fn effective_topic_config(&self, topic: &str) -> ProducerTopicConfig {
        let mut base = ProducerTopicConfig {
            compression: self.compression.clone(),
            max_message_bytes: self.batch_max_bytes(),
            linger_ms: self.linger_ms,
            ..Default::default()
        };

        if let Some(ov) = self.topics.get(topic) {
            if !ov.compression.is_empty() {
                base.compression.clone_from(&ov.compression);
            }
            if ov.max_message_bytes > 0 {
                base.max_message_bytes = ov.max_message_bytes;
            }
            if ov.linger_ms > 0 {
                base.linger_ms = ov.linger_ms;
            }
            if ov.batch_size > 0 {
                base.batch_size = ov.batch_size;
            }
            if !ov.acks.is_empty() {
                base.acks.clone_from(&ov.acks);
            }
            if ov.retries > 0 {
                base.retries = ov.retries;
            }
            if ov.retry_backoff_ms > 0 {
                base.retry_backoff_ms = ov.retry_backoff_ms;
            }
        }
        base
    }

    /// Returns `batch_max_bytes` with a 1 MB default.
    #[allow(clippy::missing_const_for_fn)]
    pub fn batch_max_bytes(&self) -> i32 {
        if self.batch_max_bytes > 0 { self.batch_max_bytes } else { 1024 * 1024 }
    }

    /// Request timeout as a [`Duration`] (default 30 s).
    ///
    /// Mirrors Go's `ProducerConfig.RequestTimeout()`.
    pub fn request_timeout(&self) -> Duration {
        if self.request_timeout_ms > 0 {
            #[allow(clippy::cast_sign_loss)]
            Duration::from_millis(self.request_timeout_ms as u64)
        } else {
            Duration::from_secs(30)
        }
    }
}

/// Per-topic consumer overrides.
///
/// Zero values are ignored; global [`ConsumerConfig`] defaults apply instead.
/// Mirrors Go's `ConsumerTopicConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsumerTopicConfig {
    #[serde(default)]
    pub group_id: String,
    #[serde(default)]
    pub auto_offset_reset: String,
    #[serde(default)]
    pub auto_commit_interval_ms: i32,
    #[serde(default)]
    pub fetch_min_bytes: i32,
    #[serde(default)]
    pub fetch_max_wait_ms: i32,
    #[serde(default)]
    pub concurrency: i32,
}

impl ConsumerTopicConfig {
    /// Auto-commit interval as a [`Duration`] (default 5 s).
    pub fn auto_commit_interval(&self) -> Duration {
        if self.auto_commit_interval_ms > 0 {
            #[allow(clippy::cast_sign_loss)]
            Duration::from_millis(self.auto_commit_interval_ms as u64)
        } else {
            Duration::from_secs(5)
        }
    }

    /// Fetch max-wait as a [`Duration`] (default 500 ms).
    pub fn fetch_max_wait(&self) -> Duration {
        if self.fetch_max_wait_ms > 0 {
            #[allow(clippy::cast_sign_loss)]
            Duration::from_millis(self.fetch_max_wait_ms as u64)
        } else {
            Duration::from_millis(500)
        }
    }
}

/// Global consumer configuration with optional per-topic overrides.
///
/// Mirrors Go's `ConsumerConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsumerConfig {
    #[serde(default)]
    pub brokers: Vec<String>,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub group_id: String,
    #[serde(default)]
    pub auto_offset_reset: String,
    #[serde(default)]
    pub default_topics: Vec<String>,
    #[serde(default)]
    pub max_poll_records: i32,
    #[serde(default)]
    pub fetch_max_wait_ms: i32,
    #[serde(default)]
    pub session_timeout_ms: i32,
    #[serde(default)]
    pub heartbeat_interval_ms: i32,
    #[serde(default)]
    pub concurrency: i32,
    #[serde(default)]
    pub fetch_max_bytes: i32,
    #[serde(default)]
    pub fetch_min_bytes: i32,
    #[serde(default)]
    pub topics: HashMap<String, ConsumerTopicConfig>,
}

impl ConsumerConfig {
    /// Merge global defaults with per-topic overrides.
    ///
    /// Mirrors Go's `ConsumerConfig.EffectiveTopicConsumerConfig`.
    pub fn effective_topic_config(&self, topic: &str) -> ConsumerTopicConfig {
        let mut base = ConsumerTopicConfig {
            group_id: self.group_id.clone(),
            auto_offset_reset: normalize_offset_reset(&self.auto_offset_reset),
            fetch_min_bytes: self.fetch_min_bytes(),
            fetch_max_wait_ms: self.fetch_max_wait_ms(),
            concurrency: self.concurrency(),
            ..Default::default()
        };

        if let Some(ov) = self.topics.get(topic) {
            if !ov.group_id.is_empty() {
                base.group_id.clone_from(&ov.group_id);
            }
            if !ov.auto_offset_reset.is_empty() {
                base.auto_offset_reset = normalize_offset_reset(&ov.auto_offset_reset);
            }
            if ov.fetch_min_bytes > 0 {
                base.fetch_min_bytes = ov.fetch_min_bytes;
            }
            if ov.fetch_max_wait_ms > 0 {
                base.fetch_max_wait_ms = ov.fetch_max_wait_ms;
            }
            if ov.concurrency > 0 {
                base.concurrency = ov.concurrency;
            }
            if ov.auto_commit_interval_ms > 0 {
                base.auto_commit_interval_ms = ov.auto_commit_interval_ms;
            }
        }
        base
    }

    /// Session timeout as a [`Duration`] (default 10 s).
    pub fn session_timeout(&self) -> Duration {
        if self.session_timeout_ms > 0 {
            #[allow(clippy::cast_sign_loss)]
            Duration::from_millis(self.session_timeout_ms as u64)
        } else {
            Duration::from_secs(10)
        }
    }

    /// Heartbeat interval as a [`Duration`] (default 3 s).
    pub fn heartbeat_interval(&self) -> Duration {
        if self.heartbeat_interval_ms > 0 {
            #[allow(clippy::cast_sign_loss)]
            Duration::from_millis(self.heartbeat_interval_ms as u64)
        } else {
            Duration::from_secs(3)
        }
    }

    /// Fetch max bytes with a 50 MB default.
    #[allow(clippy::missing_const_for_fn)]
    pub fn fetch_max_bytes_or_default(&self) -> i32 {
        if self.fetch_max_bytes > 0 { self.fetch_max_bytes } else { 50 * 1024 * 1024 }
    }

    /// Max poll records with a 100 default.
    #[allow(clippy::missing_const_for_fn)]
    pub fn max_poll_records_or_default(&self) -> i32 {
        if self.max_poll_records > 0 { self.max_poll_records } else { 100 }
    }

    #[allow(clippy::missing_const_for_fn)]
    fn fetch_max_wait_ms(&self) -> i32 {
        if self.fetch_max_wait_ms > 0 { self.fetch_max_wait_ms } else { 500 }
    }

    #[allow(clippy::missing_const_for_fn)]
    fn fetch_min_bytes(&self) -> i32 {
        if self.fetch_min_bytes > 0 { self.fetch_min_bytes } else { 1 }
    }

    #[allow(clippy::missing_const_for_fn)]
    fn concurrency(&self) -> i32 {
        if self.concurrency > 0 { self.concurrency } else { 1 }
    }
}

/// Root Kafka configuration.
///
/// Mirrors Go's `kafka.Config` (mapped from `[kafka]` in TOML).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KafkaConfig {
    #[serde(default)]
    pub producer: ProducerConfig,
    #[serde(default)]
    pub consumer: ConsumerConfig,
}

/// Maps `"newest"` / `"oldest"` to canonical `"latest"` / `"earliest"`.
///
/// Mirrors Go's `normalizeOffsetReset`.
fn normalize_offset_reset(v: &str) -> String {
    match v {
        "newest" | "" => "latest".into(),
        "oldest" => "earliest".into(),
        other => other.into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// topic.go — Topic name constants and message payload types
// ─────────────────────────────────────────────────────────────────────────────

/// Mirrors Go's `TopicBiPromotionRank`.
pub const TOPIC_BI_PROMOTION_RANK: &str = "bi-rank-promotion-event";
/// Mirrors Go's `TopicRankEvent`.
pub const TOPIC_RANK_EVENT: &str = "rank-event";
/// Mirrors Go's `TopicCohortInitializationEvent` — new user registration push.
pub const TOPIC_COHORT_INITIALIZATION_EVENT: &str = "cohort-initialization-event";
/// Mirrors Go's `TopicTaskUpdateEvent` — task completion.
pub const TOPIC_TASK_UPDATE_EVENT: &str = "task-update-event";
/// Mirrors Go's `TopicPropEvent` — send prop.
pub const TOPIC_PROP_EVENT: &str = "prop-event";
/// Mirrors Go's `TopicPurchaseEvent`.
pub const TOPIC_PURCHASE_EVENT: &str = "purchase-event";
/// Mirrors Go's `TopicTaskProgressesEvent` — task progress.
pub const TOPIC_TASK_PROGRESSES_EVENT: &str = "task-progresses-event";

/// Mirrors Go's `TopicUcsFe`.
pub const TOPIC_UCS_FE: &str = "tcg-ucs-fe";
/// Mirrors Go's `TopicUcsFeEvents`.
pub const TOPIC_UCS_FE_EVENTS: &str = "ucs-fe-events";
/// Mirrors Go's `TopicMcsSuccessPlayerDeposit`.
pub const TOPIC_MCS_SUCCESS_PLAYER_DEPOSIT: &str = "mcs_success_player_deposit";

/// Payload for [`TOPIC_TASK_UPDATE_EVENT`].
///
/// Mirrors Go's `TaskUpdate`.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TaskUpdate {
    pub message: String,
    pub task_type: i64,
    pub task_id: i64,
    pub task_item_id: i64,
    pub task_item_progresses_id: i64,
}

/// Payload for [`TOPIC_PROP_EVENT`].
///
/// Mirrors Go's `SendProp`.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct SendProp {
    pub send_user_id: i64,
    pub receive_user_id: i64,
    pub prop_id: i64,
    /// 0=none 1=line 2=parabola 3=spiral
    pub track_type: i64,
}

/// Payload for [`TOPIC_PURCHASE_EVENT`] error cases.
///
/// Mirrors Go's `BuyErrorInfo`.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct BuyErrorInfo {
    pub purchase_key: String,
    pub price: f64,
    pub coins: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// producer.go / consumer.go — Client stubs
//
// A real implementation requires a Kafka client crate (e.g. `rdkafka`).
// Add the dependency to Cargo.toml and replace the stub bodies.
// ─────────────────────────────────────────────────────────────────────────────

/// Non-200 / transport error returned by Kafka operations.
#[derive(Debug, thiserror::Error)]
pub enum KafkaError {
    #[error("Kafka producer not initialized")]
    ProducerNotInitialized,
    #[error("Kafka consumer not initialized")]
    ConsumerNotInitialized,
    #[error("Kafka error: {0}")]
    Other(String),
}

/// Kafka message record (simplified equivalent of `kgo.Record`).
#[derive(Debug, Clone)]
pub struct Record {
    pub topic: String,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub partition: i32,
    pub offset: i64,
}

/// Kafka producer stub.
///
/// Mirrors Go's `Producer` from `pkg/kafka/producer.go`.
/// Replace stub body with `rdkafka::producer::FutureProducer` when the crate
/// is added to `Cargo.toml`.
pub struct Producer {
    cfg: ProducerConfig,
}

impl Producer {
    /// Create a new `Producer` from config.
    ///
    /// Returns `None` if the broker list is empty (mirrors Go's `NewProducer` nil return).
    pub fn new(cfg: &ProducerConfig) -> Option<Self> {
        if cfg.brokers.is_empty() {
            tracing::error!("kafka producer config is invalid — no brokers");
            return None;
        }
        tracing::info!(
            brokers = ?cfg.brokers,
            client_id = %cfg.client_id,
            topic = %cfg.topic,
            "kafka producer initialized (stub)"
        );
        Some(Self { cfg: cfg.clone() })
    }

    /// Produce a message to the default topic.
    ///
    /// Stub — returns [`KafkaError::ProducerNotInitialized`] until a real Kafka
    /// client crate is wired in.
    #[allow(clippy::unused_async)]
    pub async fn produce(&self, key: &[u8], value: &[u8]) -> Result<(), KafkaError> {
        tracing::debug!(
            topic = %self.cfg.topic,
            key_len = key.len(),
            value_len = value.len(),
            "kafka produce (stub — not delivered)"
        );
        Err(KafkaError::ProducerNotInitialized)
    }

    /// Produce a message to an explicit topic.
    #[allow(clippy::unused_async)]
    pub async fn produce_to(
        &self,
        topic: &str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), KafkaError> {
        tracing::debug!(
            topic,
            key_len = key.len(),
            value_len = value.len(),
            "kafka produce_to (stub — not delivered)"
        );
        Err(KafkaError::ProducerNotInitialized)
    }

    #[allow(clippy::unused_self)]
    pub fn close(&self) {
        tracing::info!("kafka producer closed (stub)");
    }
}

/// Message handler callback type, equivalent to Go's `func(record *kgo.Record)`.
pub type MessageHandler = Box<dyn Fn(Record) + Send + Sync + 'static>;

/// Kafka consumer stub.
///
/// Mirrors Go's `Consumer` from `pkg/kafka/consumer.go`.
/// Replace stub body with `rdkafka::consumer::StreamConsumer` when the crate
/// is added to `Cargo.toml`.
pub struct Consumer {
    cfg: ConsumerConfig,
}

impl Consumer {
    /// Create a new `Consumer` from config.
    ///
    /// Returns `None` if the broker list is empty (mirrors Go's `NewConsumer` nil return).
    pub fn new(cfg: &ConsumerConfig) -> Option<Self> {
        if cfg.brokers.is_empty() {
            tracing::error!("kafka consumer config is invalid — no brokers");
            return None;
        }
        tracing::info!(
            brokers = ?cfg.brokers,
            client_id = %cfg.client_id,
            "kafka consumer initialized (stub)"
        );
        Some(Self { cfg: cfg.clone() })
    }

    /// Subscribe to a topic with the given handler.
    ///
    /// Stub — logs the subscription request and returns an error until a real
    /// Kafka crate is wired in.
    ///
    /// Mirrors Go's `Consumer.SubscribeTopic`.
    #[allow(clippy::unused_async)]
    pub async fn subscribe_topic(
        &self,
        topic: &str,
        _handler: MessageHandler,
    ) -> Result<(), KafkaError> {
        let topic_cfg = self.cfg.effective_topic_config(topic);
        tracing::info!(
            topic,
            group_id = %topic_cfg.group_id,
            concurrency = topic_cfg.concurrency,
            "kafka subscribe_topic (stub — not consuming)"
        );
        Err(KafkaError::ConsumerNotInitialized)
    }

    #[allow(clippy::unused_async)]
    pub async fn close(&self) {
        tracing::info!("kafka consumer closed (stub)");
    }
}

/// Root Kafka client holder, equivalent to Go's `KafkaProducer` / `KafkaConsumer` globals.
pub struct KafkaClients {
    pub producer: Option<Producer>,
    pub consumer: Option<Consumer>,
}

impl KafkaClients {
    /// Initialise producer and consumer from config.
    ///
    /// Mirrors Go's `Config.InitProducer()` + `Config.InitConsumer()`.
    pub fn init(cfg: &KafkaConfig) -> Self {
        let producer =
            if cfg.producer.brokers.is_empty() { None } else { Producer::new(&cfg.producer) };
        let consumer =
            if cfg.consumer.brokers.is_empty() { None } else { Consumer::new(&cfg.consumer) };
        Self { producer, consumer }
    }
}
