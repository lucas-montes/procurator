use std::{net::Ipv4Addr, str::FromStr};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};

#[derive(Clone, Debug)]
pub struct Database(SqlitePool);

impl Database {
    pub async fn new(filename: &str) -> Self {
        let connection = SqliteConnectOptions::from_str(filename)
            .expect("bad path to sqlite")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePool::connect_with(connection)
            .await
            .expect("we should conenct to the db");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("failed to run worker database migrations");

        Self(pool)
    }

    //TODO: all this IP logic shouldn't be here

    /// Mark an existing free slot as used, or insert a new slot for a fresh IP.
    pub async fn reserve_ip(
        &self,
        vm_id: &str,
        ip: Ipv4Addr,
        mac: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO ip_leases (ip_value, ip, mac, vm_id)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(ip_value) DO UPDATE SET vm_id = ?4, mac = ?3",
        )
        .bind(i64::from(u32::from(ip)))
        .bind(ip.to_string())
        .bind(mac)
        .bind(vm_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// Mark the slot as free (`is_used` = 0) so it can be reused by the next VM.
    pub async fn release_ip(&self, vm_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE ip_leases SET vm_id = NULL WHERE vm_id = ?1")
            .bind(vm_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    /// Return a free previously-used IP slot if one exists (lowest `ip_value` first).
    pub async fn first_free_ip(&self) -> Result<Option<Ipv4Addr>, sqlx::Error> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT ip FROM ip_leases WHERE vm_id IS NULL ORDER BY ip_value ASC LIMIT 1",
        )
        .fetch_optional(&self.0)
        .await?;

        Ok(row.map(|ip| ip.parse().expect("we should only be saving valid ips")))
    }

    /// Return the highest ever-allocated IP, or `None` if the table is empty.
    pub async fn last_reserved_ip(&self) -> Result<Option<Ipv4Addr>, sqlx::Error> {
        let row: Option<String> =
            sqlx::query_scalar("SELECT ip FROM ip_leases ORDER BY ip_value DESC LIMIT 1")
                .fetch_optional(&self.0)
                .await?;
        Ok(row.map(|ip| ip.parse().expect("we should only be saving valid ips")))
    }
}
