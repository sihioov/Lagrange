impl crate::common::ScratchDb {
    pub async fn writer_with_lock_timeout(&self) -> sqlx::PgPool {
        crate::common::effective_role_pool(
            &self.supervisor_url,
            &self.name,
            "research_writer",
            true,
            1,
        )
        .await
        .expect("timed research_writer pool")
    }
}
