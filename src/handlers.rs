use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{AppState, db::Database, error::ApiError, query::QueryOptions};

const TOTAL_COUNT_HEADER: &str = "X-Total-Count";

pub async fn info(State(state): State<AppState>) -> Json<Value> {
    let database = state.db.read().await;

    let resources: Map<String, Value> = database
        .root
        .iter()
        .map(|(name, value)| {
            let info = match value {
                Value::Array(items) => json!({ "type": "collection", "count": items.len() }),
                Value::Object(_) => json!({ "type": "singular" }),
                _ => json!({ "type": "invalid" }),
            };

            (name.clone(), info)
        })
        .collect();

    Json(json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "resources": resources,
    }))
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn full_db(State(state): State<AppState>) -> Json<Value> {
    let database = state.db.read().await;

    Json(Value::Object(database.root.clone()))
}

pub async fn list(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let database = state.db.read().await;

    let value = database
        .root
        .get(&resource)
        .ok_or_else(|| ApiError::not_found(format!("resource '{resource}' not found")))?;

    match value {
        Value::Array(items) => {
            let options = QueryOptions::parse(params);
            let filtered = options.apply(items);
            let total = filtered.len();

            let body = if let Some(page) = options.page {
                let per_page = options
                    .per_page
                    .or(options.limit)
                    .filter(|size| *size > 0)
                    .unwrap_or(10);
                let pages = total.div_ceil(per_page);
                let start = page.saturating_sub(1).saturating_mul(per_page);
                let end = start.saturating_add(per_page).min(total);
                let data = if start >= total {
                    Vec::new()
                } else {
                    filtered[start..end].to_vec()
                };

                let previous = if page > 1 {
                    Value::from((page - 1) as u64)
                } else {
                    Value::Null
                };
                let next = if page < pages {
                    Value::from((page + 1) as u64)
                } else {
                    Value::Null
                };

                json!({
                    "first": 1,
                    "prev": previous,
                    "next": next,
                    "last": pages,
                    "pages": pages,
                    "items": total,
                    "data": data,
                })
            } else {
                Value::Array(options.slice(filtered))
            };

            let mut response = Json(body).into_response();
            response.headers_mut().insert(
                TOTAL_COUNT_HEADER,
                HeaderValue::from_str(&total.to_string())
                    .expect("item count is a valid header value"),
            );

            Ok(response)
        }
        Value::Object(object) => Ok(Json(Value::Object(object.clone())).into_response()),
        _ => Err(ApiError::internal(format!(
            "resource '{resource}' must be an array or an object"
        ))),
    }
}

pub async fn get_one(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let database = state.db.read().await;
    let items = collection(&database.root, &resource)?;

    let item = items
        .iter()
        .find(|item| id_matches(item, &id))
        .ok_or_else(|| ApiError::not_found(format!("{resource}/{id} not found")))?;

    Ok(Json(item.clone()))
}

pub async fn create(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let mut database = state.db.write().await;

    let existing = collection(&database.root, &resource)?.clone();

    let Value::Object(mut object) = body else {
        return Err(ApiError::bad_request("request body must be a JSON object"));
    };

    match object.get("id") {
        None | Some(Value::Null) => {
            object.insert("id".to_string(), next_id(&existing));
        }
        Some(id) => {
            let id = id_to_string(id);

            if existing.iter().any(|item| id_matches(item, &id)) {
                return Err(ApiError::conflict(format!(
                    "an item with id '{id}' already exists in '{resource}'"
                )));
            }
        }
    }

    let item = Value::Object(object);
    let mut root = database.root.clone();
    let items = root
        .get_mut(&resource)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ApiError::internal(format!("resource '{resource}' is not an array")))?;

    items.push(item.clone());

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok((StatusCode::CREATED, Json(item)).into_response())
}

pub async fn update(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut database = state.db.write().await;

    let Value::Object(mut object) = body else {
        return Err(ApiError::bad_request("request body must be a JSON object"));
    };

    let mut root = database.root.clone();
    let items = match root.get_mut(&resource) {
        Some(Value::Array(items)) => items,
        Some(Value::Object(_)) => {
            return Err(ApiError::method_not_allowed(format!(
                "'{resource}' is a singular resource, use PUT /{resource}"
            )));
        }
        Some(_) => {
            return Err(ApiError::internal(format!(
                "resource '{resource}' must be an array or an object"
            )));
        }
        None => {
            return Err(ApiError::not_found(format!(
                "resource '{resource}' not found"
            )));
        }
    };

    let index = items
        .iter()
        .position(|item| id_matches(item, &id))
        .ok_or_else(|| ApiError::not_found(format!("{resource}/{id} not found")))?;

    let original_id = items[index]
        .get("id")
        .cloned()
        .unwrap_or_else(|| Value::String(id.clone()));
    object.insert("id".to_string(), original_id);

    let item = Value::Object(object);
    items[index] = item.clone();

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok(Json(item))
}

pub async fn patch(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut database = state.db.write().await;

    let Value::Object(patch) = body else {
        return Err(ApiError::bad_request("request body must be a JSON object"));
    };

    let mut root = database.root.clone();
    let items = match root.get_mut(&resource) {
        Some(Value::Array(items)) => items,
        Some(Value::Object(_)) => {
            return Err(ApiError::method_not_allowed(format!(
                "'{resource}' is a singular resource, use PATCH /{resource}"
            )));
        }
        Some(_) => {
            return Err(ApiError::internal(format!(
                "resource '{resource}' must be an array or an object"
            )));
        }
        None => {
            return Err(ApiError::not_found(format!(
                "resource '{resource}' not found"
            )));
        }
    };

    let index = items
        .iter()
        .position(|item| id_matches(item, &id))
        .ok_or_else(|| ApiError::not_found(format!("{resource}/{id} not found")))?;

    let Value::Object(existing) = &mut items[index] else {
        return Err(ApiError::internal(format!(
            "item {resource}/{id} is not an object"
        )));
    };

    for (key, value) in patch {
        if key != "id" {
            existing.insert(key, value);
        }
    }

    let item = items[index].clone();

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok(Json(item))
}

pub async fn delete_one(
    State(state): State<AppState>,
    Path((resource, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let mut database = state.db.write().await;

    let mut root = database.root.clone();
    let items = match root.get_mut(&resource) {
        Some(Value::Array(items)) => items,
        Some(Value::Object(_)) => {
            return Err(ApiError::method_not_allowed(format!(
                "'{resource}' is a singular resource"
            )));
        }
        Some(_) => {
            return Err(ApiError::internal(format!(
                "resource '{resource}' must be an array or an object"
            )));
        }
        None => {
            return Err(ApiError::not_found(format!(
                "resource '{resource}' not found"
            )));
        }
    };

    let index = items
        .iter()
        .position(|item| id_matches(item, &id))
        .ok_or_else(|| ApiError::not_found(format!("{resource}/{id} not found")))?;

    items.remove(index);

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok(Json(json!({})))
}

pub async fn put_singular(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut database = state.db.write().await;

    match database.root.get(&resource) {
        Some(Value::Object(_)) => {}
        Some(Value::Array(_)) => {
            return Err(ApiError::method_not_allowed(format!(
                "'{resource}' is a collection, use PUT /{resource}/:id"
            )));
        }
        Some(_) => {
            return Err(ApiError::internal(format!(
                "resource '{resource}' must be an array or an object"
            )));
        }
        None => {
            return Err(ApiError::not_found(format!(
                "resource '{resource}' not found"
            )));
        }
    }

    if !body.is_object() {
        return Err(ApiError::bad_request("request body must be a JSON object"));
    }

    let mut root = database.root.clone();
    root.insert(resource, body.clone());

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok(Json(body))
}

pub async fn patch_singular(
    State(state): State<AppState>,
    Path(resource): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut database = state.db.write().await;

    let Value::Object(patch) = body else {
        return Err(ApiError::bad_request("request body must be a JSON object"));
    };

    let mut root = database.root.clone();
    let existing = match root.get_mut(&resource) {
        Some(Value::Object(existing)) => existing,
        Some(Value::Array(_)) => {
            return Err(ApiError::method_not_allowed(format!(
                "'{resource}' is a collection, use PATCH /{resource}/:id"
            )));
        }
        Some(_) => {
            return Err(ApiError::internal(format!(
                "resource '{resource}' must be an array or an object"
            )));
        }
        None => {
            return Err(ApiError::not_found(format!(
                "resource '{resource}' not found"
            )));
        }
    };

    for (key, value) in patch {
        existing.insert(key, value);
    }

    let item = Value::Object(existing.clone());

    Database::persist(&database.file, &root).await?;
    database.root = root;

    Ok(Json(item))
}

fn collection<'a>(
    root: &'a Map<String, Value>,
    resource: &str,
) -> Result<&'a Vec<Value>, ApiError> {
    match root.get(resource) {
        Some(Value::Array(items)) => Ok(items),
        Some(Value::Object(_)) => Err(ApiError::method_not_allowed(format!(
            "'{resource}' is a singular resource"
        ))),
        Some(_) => Err(ApiError::internal(format!(
            "resource '{resource}' must be an array or an object"
        ))),
        None => Err(ApiError::not_found(format!(
            "resource '{resource}' not found"
        ))),
    }
}

fn id_matches(item: &Value, id: &str) -> bool {
    match item.get("id") {
        Some(Value::String(value)) => value == id,
        Some(Value::Number(value)) => value
            .as_f64()
            .and_then(|value| id.parse::<f64>().ok().map(|id| id == value))
            .unwrap_or_else(|| value.to_string() == id),
        Some(Value::Bool(_))
        | Some(Value::Null)
        | Some(Value::Array(_))
        | Some(Value::Object(_)) => false,
        None => false,
    }
}

fn id_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        value => value.to_string(),
    }
}

fn next_id(items: &[Value]) -> Value {
    let mut highest = 0u64;
    let mut has_string_id = false;

    for item in items {
        match item.get("id") {
            Some(Value::Number(value)) => {
                if let Some(value) = value.as_u64() {
                    highest = highest.max(value);
                }
            }
            Some(Value::String(_)) => has_string_id = true,
            _ => {}
        }
    }

    if has_string_id {
        Value::String(Uuid::new_v4().to_string())
    } else {
        Value::from(highest + 1)
    }
}
