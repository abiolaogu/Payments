-- Nvuto Bookings payment lifecycle ownership tables.
-- Payments owns monetary truth; Bookings stores only external references and workflow state.

CREATE TABLE IF NOT EXISTS booking_payment_holds (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(128) NOT NULL,
    external_reference VARCHAR(128) NOT NULL UNIQUE,
    booking_reference VARCHAR(128) NOT NULL,
    amount DECIMAL(20,4) NOT NULL CHECK (amount > 0),
    currency VARCHAR(3) NOT NULL CHECK (char_length(currency) = 3),
    captured_amount DECIMAL(20,4) NOT NULL DEFAULT 0 CHECK (captured_amount >= 0),
    refunded_amount DECIMAL(20,4) NOT NULL DEFAULT 0 CHECK (refunded_amount >= 0),
    state VARCHAR(32) NOT NULL CHECK (state IN ('AUTHORIZED','CAPTURED','RELEASED','PARTIALLY_REFUNDED','REFUNDED','DECLINED','EXPIRED')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT booking_payment_hold_capture_bound CHECK (captured_amount <= amount),
    CONSTRAINT booking_payment_hold_refund_bound CHECK (refunded_amount <= captured_amount),
    CONSTRAINT booking_payment_hold_tenant_booking_unique UNIQUE (tenant_id, booking_reference)
);

CREATE INDEX IF NOT EXISTS idx_booking_payment_holds_tenant_booking
    ON booking_payment_holds (tenant_id, booking_reference);
CREATE INDEX IF NOT EXISTS idx_booking_payment_holds_tenant_state
    ON booking_payment_holds (tenant_id, state);

CREATE TABLE IF NOT EXISTS booking_payment_idempotency (
    tenant_id VARCHAR(128) NOT NULL,
    idempotency_key VARCHAR(200) NOT NULL,
    operation VARCHAR(32) NOT NULL,
    fingerprint VARCHAR(512) NOT NULL,
    response JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_booking_payment_idempotency_created
    ON booking_payment_idempotency (created_at);

CREATE TABLE IF NOT EXISTS booking_payment_event_outbox (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(128) NOT NULL,
    aggregate_reference VARCHAR(128) NOT NULL,
    event_type VARCHAR(96) NOT NULL,
    payload JSONB NOT NULL,
    correlation_id VARCHAR(128),
    causation_id VARCHAR(128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_booking_payment_event_outbox_unpublished
    ON booking_payment_event_outbox (created_at)
    WHERE published_at IS NULL;
