use std::{cmp::Ordering, collections::HashMap};

use serde_json::Value;

#[derive(Debug, Default)]
pub struct QueryOptions {
    pub filters: Vec<(String, String)>,
    pub sort: Vec<String>,
    pub order: Vec<String>,
    pub search: Option<String>,
    pub start: Option<usize>,
    pub limit: Option<usize>,
    pub page: Option<usize>,
    pub per_page: Option<usize>,
}

impl QueryOptions {
    pub fn parse(params: HashMap<String, String>) -> Self {
        let mut options = Self::default();

        for (key, value) in params {
            match key.as_str() {
                "q" => options.search = Some(value),
                "_sort" => options.sort = split_list(&value),
                "_order" => options.order = split_list(&value),
                "_start" => options.start = value.parse().ok(),
                "_limit" => options.limit = value.parse().ok(),
                "_page" => options.page = value.parse().ok(),
                "_per_page" => options.per_page = value.parse().ok(),
                _ if key.starts_with('_') => {}
                _ => options.filters.push((key, value)),
            }
        }

        options
    }

    pub fn apply(&self, items: &[Value]) -> Vec<Value> {
        let mut items: Vec<Value> = items
            .iter()
            .filter(|item| self.matches_search(item))
            .filter(|item| {
                self.filters
                    .iter()
                    .all(|(field, value)| matches_filter(item, field, value))
            })
            .cloned()
            .collect();

        if !self.sort.is_empty() {
            sort_items(&mut items, &self.sort, &self.order);
        }

        items
    }

    pub fn slice(&self, items: Vec<Value>) -> Vec<Value> {
        let total = items.len();
        let start = self.start.unwrap_or(0).min(total);
        let end = self
            .limit
            .map(|limit| start.saturating_add(limit))
            .unwrap_or(total)
            .min(total);

        items[start..end].to_vec()
    }

    fn matches_search(&self, item: &Value) -> bool {
        match &self.search {
            Some(search) => contains_text(item, &search.to_lowercase()),
            None => true,
        }
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.to_lowercase().contains(needle),
        Value::Number(value) => value.to_string().contains(needle),
        Value::Bool(value) => value.to_string().contains(needle),
        Value::Array(values) => values.iter().any(|value| contains_text(value, needle)),
        Value::Object(values) => values.values().any(|value| contains_text(value, needle)),
        Value::Null => false,
    }
}

#[derive(Clone, Copy)]
enum Operator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

fn matches_filter(item: &Value, key: &str, raw: &str) -> bool {
    let (field, operator) = parse_operator(key);
    let Some(actual) = get_path(item, field) else {
        return false;
    };

    match operator {
        Operator::Equal => raw
            .split(',')
            .any(|candidate| value_equals(actual, candidate.trim())),
        Operator::NotEqual => !raw
            .split(',')
            .any(|candidate| value_equals(actual, candidate.trim())),
        Operator::GreaterThan => compare_value(actual, raw)
            .map(|ordering| ordering == Ordering::Greater)
            .unwrap_or(false),
        Operator::GreaterThanOrEqual => compare_value(actual, raw)
            .map(|ordering| ordering != Ordering::Less)
            .unwrap_or(false),
        Operator::LessThan => compare_value(actual, raw)
            .map(|ordering| ordering == Ordering::Less)
            .unwrap_or(false),
        Operator::LessThanOrEqual => compare_value(actual, raw)
            .map(|ordering| ordering != Ordering::Greater)
            .unwrap_or(false),
    }
}

fn parse_operator(key: &str) -> (&str, Operator) {
    let operators = [
        ("_gte", Operator::GreaterThanOrEqual),
        ("_lte", Operator::LessThanOrEqual),
        ("_ne", Operator::NotEqual),
        ("_gt", Operator::GreaterThan),
        ("_lt", Operator::LessThan),
    ];

    for (suffix, operator) in operators {
        if let Some(field) = key.strip_suffix(suffix) {
            return (field, operator);
        }
    }

    (key, Operator::Equal)
}

fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;

    for segment in path.split('.') {
        current = match current {
            Value::Object(object) => object.get(segment)?,
            Value::Array(array) => array.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    Some(current)
}

fn value_equals(actual: &Value, raw: &str) -> bool {
    match actual {
        Value::String(value) => value == raw,
        Value::Bool(value) => raw
            .parse::<bool>()
            .map(|raw| raw == *value)
            .unwrap_or(false),
        Value::Number(value) => raw
            .parse::<f64>()
            .ok()
            .and_then(|raw| value.as_f64().map(|value| value == raw))
            .unwrap_or(false),
        Value::Null => raw == "null",
        Value::Array(values) => values.iter().any(|value| value_equals(value, raw)),
        Value::Object(_) => false,
    }
}

fn compare_value(actual: &Value, raw: &str) -> Option<Ordering> {
    match actual {
        Value::Number(value) => {
            let raw = raw.parse::<f64>().ok()?;
            value.as_f64()?.partial_cmp(&raw)
        }
        Value::String(value) => Some(value.as_str().cmp(raw)),
        Value::Bool(value) => {
            let raw = raw.parse::<bool>().ok()?;
            Some(value.cmp(&raw))
        }
        _ => None,
    }
}

fn sort_items(items: &mut [Value], fields: &[String], orders: &[String]) {
    items.sort_by(|left, right| {
        for (index, field) in fields.iter().enumerate() {
            let left_value = get_path(left, field).unwrap_or(&Value::Null);
            let right_value = get_path(right, field).unwrap_or(&Value::Null);
            let mut ordering = compare_json(left_value, right_value);

            if orders
                .get(index)
                .map(|order| order.eq_ignore_ascii_case("desc"))
                .unwrap_or(false)
            {
                ordering = ordering.reverse();
            }

            if ordering != Ordering::Equal {
                return ordering;
            }
        }

        Ordering::Equal
    });
}

fn compare_json(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .and_then(|left| right.as_f64().and_then(|right| left.partial_cmp(&right)))
            .unwrap_or(Ordering::Equal),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Array(left), Value::Array(right)) => left.len().cmp(&right.len()),
        (Value::Object(_), Value::Object(_)) => Ordering::Equal,
        _ => type_rank(left).cmp(&type_rank(right)),
    }
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}
