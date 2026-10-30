use insight::error::{ErrorKind, Result};
use insight_schema::log::LogRecord;
use serde_json::{Value, json};

#[derive(Debug)]
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn bounded(&mut self, upper: u32) -> u32 {
        let [a, b, c, d, _, _, _, _] = self.next().to_le_bytes();
        u32::from_le_bytes([a, b, c, d]) % upper
    }

    fn id(&mut self) -> String {
        format!("{:016x}{:016x}", self.next(), self.next())
    }

    fn span(&mut self) -> String {
        format!("{:016x}", self.next())
    }

    fn context(&mut self) -> [String; 3] {
        [
            format!("tenant-{:04}", 1 + self.bounded(64)),
            format!("customer-{:06}", 1 + self.bounded(100_000)),
            format!("session-{:016x}", self.next()),
        ]
    }
}

#[derive(Debug)]
pub(super) struct Generator {
    random: Random,
    watermark: i64,
    pending: Vec<LogRecord>,
    pub(super) query_words: [String; 3],
    pub(super) trace_id: String,
}

#[derive(Debug)]
struct Request {
    start: i64,
    trace: String,
    id: String,
    span: String,
    context: [String; 3],
    client_ip: String,
    client: &'static str,
}

impl Generator {
    pub(super) fn new(seed: u64) -> Self {
        let mut random = Random(seed.max(1));
        let query_words = random.context();
        let trace_id = random.id();
        Self {
            random,
            watermark: 0,
            pending: Vec::new(),
            query_words,
            trace_id,
        }
    }

    pub(super) fn window(&mut self) -> Result<Vec<LogRecord>> {
        let next = self
            .watermark
            .checked_add(1_000_000_000)
            .ok_or_else(|| ErrorKind::Invariant("log request timeline overflowed".to_owned()))?;
        next.checked_add(5_000_000_000)
            .ok_or_else(|| ErrorKind::Invariant("log event timeline overflowed".to_owned()))?;
        for slot in 0..1_024 {
            let first = self.watermark == 0 && slot == 0;
            let offset =
                i64::from(slot) * 1_000_000_000 / 1_024 + i64::from(self.random.bounded(900_000));
            let request = Request {
                start: if first {
                    0
                } else {
                    self.watermark + offset.max(1_000_000)
                },
                trace: if first {
                    self.trace_id.clone()
                } else {
                    self.random.id()
                },
                id: self.random.span(),
                span: self.random.span(),
                context: if first || self.random.bounded(128) == 0 {
                    self.query_words.clone()
                } else {
                    self.random.context()
                },
                client_ip: format!("203.0.113.{}", 1 + self.random.bounded(254)),
                client: match self.random.bounded(5) {
                    0 => "commerce-ios",
                    1 => "commerce-android",
                    _ => "commerce-web",
                },
            };
            request.generate(&mut self.random, &mut self.pending, first);
        }
        self.pending.sort_by_key(|record| record.timestamp);
        let ready = self
            .pending
            .partition_point(|record| record.timestamp < next);
        self.watermark = next;
        Ok(self.pending.drain(..ready).collect())
    }
}

impl Request {
    fn record(
        &self,
        service: &str,
        event: &str,
        span: (&str, Option<&str>),
        elapsed_ms: u32,
        attributes: &Value,
    ) -> LogRecord {
        let level = match event {
            "job.failed" => "ERROR",
            "payment.timeout" | "inventory.rejected" | "order.rejected" => "WARN",
            _ => "INFO",
        };
        let (container, node) = match service {
            "gateway" => ("a80c49e236d1", "commerce-node-1"),
            "checkout" => ("b19e638a204f", "commerce-node-2"),
            "inventory" => ("c72d490e86ab", "commerce-node-3"),
            "payments" => ("d30e7a891c65", "commerce-node-2"),
            _ => ("e49c702d16ab", "commerce-node-3"),
        };
        let [tenant, customer, session] = &self.context;
        LogRecord {
            timestamp: self.start + i64::from(elapsed_ms) * 1_000_000,
            source: if level == "INFO" { "stdout" } else { "stderr" }.to_owned(),
            container_id: container.to_owned(),
            node_id: node.to_owned(),
            log: json!({
                "level": level, "service": service, "event": event,
                "trace_id": self.trace, "span_id": span.0, "parent_span_id": span.1,
                "request_id": self.id, "tenant_id": tenant, "customer_id": customer,
                "session_id": session, "attributes": attributes,
            })
            .to_string(),
        }
    }

    fn generate(&self, random: &mut Random, records: &mut Vec<LogRecord>, first: bool) {
        let browse = first || random.bounded(100) < 90;
        let sku = 1 + random.bounded(5_000);
        let (method, path, status, elapsed, bytes) = if browse {
            (
                "GET",
                format!("/api/v1/products/SKU-{sku:06}"),
                200,
                if first { 1 } else { 1 + random.bounded(80) },
                700 + random.bounded(2_300),
            )
        } else {
            let (status, elapsed) = self.checkout(random, records, sku);
            ("POST", "/api/v1/orders".to_owned(), status, elapsed, 280)
        };
        records.push(self.record(
            "gateway",
            "http.access",
            (&self.span, None),
            elapsed,
            &json!({
                "method": method, "path": path, "status": status, "duration_ms": elapsed,
                "response_bytes": bytes, "client_ip": self.client_ip, "client": self.client,
            }),
        ));
    }

    fn checkout(&self, random: &mut Random, records: &mut Vec<LogRecord>, sku: u32) -> (u16, u32) {
        let scenario = random.bounded(100);
        let order = format!("order-{}", random.span());
        let span = random.span();
        let child = (span.as_str(), Some(self.span.as_str()));
        let inventory_span = random.span();
        let inventory_end = 3 + random.bounded(25);
        let quantity = 1 + random.bounded(3);
        let price = match sku % 5 {
            0 => 1_299,
            1 => 2_499,
            2 => 12_999,
            3 => 6_999,
            _ => 3_999,
        };
        let amount = quantity * price;
        let sku = format!("SKU-{sku:06}");
        if scenario < 2 {
            records.push(self.record(
                "inventory",
                "inventory.rejected",
                (&inventory_span, Some(&span)),
                inventory_end,
                &json!({
                    "order_id": order, "sku": sku, "requested_quantity": quantity,
                    "available_quantity": 0, "reserved_quantity": 0,
                    "reason": "insufficient_stock", "duration_ms": inventory_end - 2,
                }),
            ));
            records.push(self.record(
                "checkout",
                "order.rejected",
                child,
                inventory_end + 1,
                &json!({
                    "order_id": order, "reason": "insufficient_stock", "payment_attempted": false,
                    "duration_ms": inventory_end - 1,
                }),
            ));
            return (409, inventory_end + 2);
        }
        let reservation = random.span();
        records.push(self.record(
            "inventory",
            "inventory.reserved",
            (&inventory_span, Some(&span)),
            inventory_end,
            &json!({
                "order_id": order, "reservation_id": reservation, "sku": sku, "quantity": quantity,
                "warehouse": "us-east-1", "duration_ms": inventory_end - 2,
            }),
        ));
        let payment = random.span();
        let mut payment_end = inventory_end + 1;
        let retry = scenario < 4;
        if retry {
            let attempt_span = random.span();
            payment_end += 2_000;
            records.push(self.record("payments", "payment.timeout", (&attempt_span, Some(&span)), payment_end, &json!({
                "order_id": order, "payment_id": payment, "idempotency_key": order,
                "provider_request_id": random.span(), "amount_cents": amount, "currency": "USD",
                "attempt": 1, "duration_ms": 2_000, "retry_after_ms": 250,
                "error": "authorization response deadline exceeded", "authorization_state": "unknown",
            })));
            payment_end += 250;
        }
        let duration = 50 + random.bounded(350);
        payment_end += duration;
        let attempt_span = random.span();
        records.push(self.record(
            "payments",
            "payment.authorized",
            (&attempt_span, Some(&span)),
            payment_end,
            &json!({
                "order_id": order, "payment_id": payment, "idempotency_key": order,
                "provider_request_id": random.span(), "amount_cents": amount, "currency": "USD",
                "attempt": if retry { 2 } else { 1 }, "duration_ms": duration,
                "authorization_id": random.span(), "capture": "manual",
            }),
        ));
        let committed = payment_end + 5 + random.bounded(20);
        let message = random.id();
        records.push(self.record(
            "checkout",
            "order.accepted",
            child,
            committed,
            &json!({
                "order_id": order, "payment_id": payment, "reservation_id": reservation,
                "message_id": message, "sku": sku, "quantity": quantity,
                "unit_price_cents": price, "amount_cents": amount, "currency": "USD",
                "outbox_state": "pending", "duration_ms": committed - 2,
            }),
        ));
        records.push(self.consume(random, &order, &message, (&span, committed)));
        (201, committed + 2)
    }

    fn consume(
        &self,
        random: &mut Random,
        order: &str,
        message: &str,
        parent: (&str, u32),
    ) -> LogRecord {
        let failed = random.bounded(100) == 0;
        let delay = 10 + random.bounded(90);
        let duration = if failed {
            1_000
        } else {
            20 + random.bounded(180)
        };
        let worker_span = random.span();
        let (event, attributes) = if failed {
            (
                "job.failed",
                json!({
                    "order_id": order, "message_id": message, "queue": "orders.created.v2",
                    "delivery_count": 1, "queue_delay_ms": delay, "duration_ms": duration,
                    "state": "retry_scheduled", "retry_after_ms": 5_000,
                    "error": {
                        "kind": "PoolTimedOut",
                        "message": "could not persist fulfillment request",
                        "causes": ["begin transaction failed", "timed out acquiring orders-primary connection"],
                        "stack": [
                            "commerce_worker::fulfillment::create at src/fulfillment.rs:84",
                            "commerce_worker::consumer::process at src/consumer.rs:112",
                            "commerce_worker::consumer::run at src/consumer.rs:67",
                        ],
                        "pool": {"max_connections": 32, "active_connections": 32, "idle_connections": 0},
                    },
                }),
            )
        } else {
            (
                "fulfillment.created",
                json!({
                    "order_id": order, "message_id": message, "queue": "orders.created.v2",
                    "delivery_count": 1, "queue_delay_ms": delay, "duration_ms": duration,
                    "fulfillment_id": random.span(),
                }),
            )
        };
        self.record(
            "order-worker",
            event,
            (&worker_span, Some(parent.0)),
            parent.1 + delay + duration,
            &attributes,
        )
    }
}
