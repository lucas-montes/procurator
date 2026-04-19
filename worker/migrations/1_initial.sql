CREATE TABLE IF NOT EXISTS ip_leases (
    ip_value   INTEGER PRIMARY KEY, -- Numeric IP; used for ordering and uniqueness.
    ip         TEXT    NOT NULL UNIQUE,
    mac        TEXT    NOT NULL UNIQUE,
    vm_id      TEXT -- NULL when the slot is free.
);
