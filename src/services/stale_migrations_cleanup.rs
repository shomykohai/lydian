use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use sqlx::Row;
use tower::Service;

use crate::AppState;
use crate::services::ServiceError;

pub struct StaleMigrationsCleanupService {
    pub state: Arc<AppState>,
}

impl StaleMigrationsCleanupService {
    pub const fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl Service<()> for StaleMigrationsCleanupService {
    type Error = ServiceError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = ();

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: ()) -> Self::Future {
        let state = self.state.clone();

        Box::pin(async move {
            let records = sqlx::query(
                r#"
                SELECT id, output_file_path
                FROM migrations
                WHERE service != 'local' AND output_file_path IS NOT NULL
                "#,
            )
            .fetch_all(&state.storage.db)
            .await?;

            let mut to_delete = Vec::with_capacity(records.len());

            for record in records {
                let path_str: Option<String> = record.get("output_file_path");

                if let Some(path_str) = path_str {
                    let path = std::path::Path::new(&path_str);

                    if !path.exists() {
                        let id: i64 = record.get("id");
                        to_delete.push(id);
                    }
                }
            }

            if !to_delete.is_empty() {
                let mut tx = state.storage.db.begin().await.map_err(ServiceError::from)?;

                for id in to_delete {
                    sqlx::query("DELETE FROM migrations WHERE id = ?")
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(ServiceError::from)?;
                }

                tx.commit().await.map_err(ServiceError::from)?;
            }

            Ok(())
        })
    }
}
