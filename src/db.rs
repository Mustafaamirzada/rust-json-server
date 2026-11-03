use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use tokio::fs;

use crate::error::ApiError;

#[derive(Debug)]
pub struct Database {
    pub file: PathBuf,
    pub root: Map<String, Value>,
}

impl Database {
    pub async fn load(file: PathBuf) -> Result<Self, ApiError> {
        let content = match fs::read_to_string(&file).await {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_parent_dir(&file).await?;
                fs::write(&file, "{}\n").await?;
                tracing::warn!(path = %file.display(), "database file not found, created an empty one");
                "{}".to_string()
            }
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "failed to read database file '{}': {error}",
                    file.display()
                )));
            }
        };

        let content = content.trim();
        let value: Value = if content.is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(content).map_err(|error| {
                ApiError::bad_request(format!("invalid JSON in '{}': {error}", file.display()))
            })?
        };

        let Value::Object(root) = value else {
            return Err(ApiError::bad_request(
                "database root must be a JSON object, e.g. { \"posts\": [] }",
            ));
        };

        Ok(Self { file, root })
    }

    pub async fn persist(file: &Path, root: &Map<String, Value>) -> Result<(), ApiError> {
        create_parent_dir(file).await?;

        let serialized = serde_json::to_string_pretty(root)?;
        let temporary = temporary_path(file);

        fs::write(&temporary, serialized).await?;
        fs::rename(&temporary, file).await?;

        Ok(())
    }
}

async fn create_parent_dir(file: &Path) -> Result<(), ApiError> {
    if let Some(parent) = file.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }

    Ok(())
}

fn temporary_path(file: &Path) -> PathBuf {
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("json");

    file.with_extension(format!("{extension}.tmp"))
}
