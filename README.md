# rustjs-server

A small, fast HTTP server that turns a JSON file into a REST API, inspired by
[json-server](https://www.npmjs.com/package/json-server). Built with Rust,
[axum](https://github.com/tokio-rs/axum), and [tokio](https://tokio.rs).

## Use cases

- **Frontend prototyping** - give a UI a real API before the backend exists.
- **Mock APIs** - replace a third-party or unfinished service during development.
- **Data exploration** - expose an existing JSON export/dataset over HTTP.
- **Demos and workshops** - ship a self-contained API with zero database setup.
- **Test fixtures** - use a JSON file as a stateful backend in integration tests.
- **Small internal tools** - lightweight CRUD store for scripts and admin panels.

## Quick start

```bash
# Start with an existing file
cargo run -- db.json

# Choose a host and port
cargo run -- db.json --host 0.0.0.0 --port 8080

# Build a release binary
cargo build --release
./target/release/rustjs-server db.json
```

If the file does not exist, it is created with an empty object (`{}`). All
writes are persisted back to the same file using an atomic temp-file rename.

## Data format

The root must be a JSON object. Arrays become collections and objects become
singular resources.

```json
{
  "posts": [
    { "id": 1, "title": "Hello world", "author": "Ada", "views": 120 },
    { "id": 2, "title": "Rust is fast", "author": "Linus", "views": 80 }
  ],
  "comments": [
    { "id": 1, "postId": 1, "body": "Nice post" }
  ],
  "profile": { "name": "Ada Lovelace", "email": "ada@example.com" }
}
```

## API

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/` | List available resources and item counts |
| `GET` | `/db` | Return the entire database |
| `GET` | `/health` | Health check |
| `GET` | `/:resource` | List a collection or fetch a singular resource |
| `GET` | `/:resource/:id` | Fetch one item by `id` |
| `POST` | `/:resource` | Create an item (auto-generates `id` when omitted) |
| `PUT` | `/:resource/:id` | Replace an item, preserving its `id` |
| `PATCH` | `/:resource/:id` | Shallow-merge fields into an item |
| `DELETE` | `/:resource/:id` | Delete an item |
| `PUT` | `/:singular` | Replace a singular resource |
| `PATCH` | `/:singular` | Shallow-merge a singular resource |

Examples:

```bash
curl http://localhost:3000/posts
curl http://localhost:3000/posts/1
curl -X POST http://localhost:3000/posts \
  -H 'Content-Type: application/json' \
  -d '{"title":"New post","author":"Grace"}'
curl -X PATCH http://localhost:3000/posts/1 \
  -H 'Content-Type: application/json' \
  -d '{"views":150}'
curl -X DELETE http://localhost:3000/posts/1
```

Full-database access:

```bash
curl http://localhost:3000/db
curl http://localhost:3000/
```

```json
{
  "name": "rustjs-server",
  "version": "0.1.0",
  "resources": {
    "posts": { "type": "collection", "count": 2 },
    "profile": { "type": "singular" }
  }
}
```

## Query parameters

### Filtering

```bash
# Exact match
curl 'http://localhost:3000/posts?author=Ada'

# Multiple values (OR)
curl 'http://localhost:3000/posts?author=Ada,Linus'

# Comparisons
curl 'http://localhost:3000/posts?views_gte=100'
curl 'http://localhost:3000/posts?views_lt=100'

# Not equal
curl 'http://localhost:3000/posts?author_ne=Ada'

# Nested fields and array membership
curl 'http://localhost:3000/posts?tags=api'
curl 'http://localhost:3000/posts?meta.category=news'
```

Supported operators: `_ne`, `_gt`, `_gte`, `_lt`, `_lte`.

### Search, sorting, and pagination

```bash
# Case-insensitive full-text search across values
curl 'http://localhost:3000/posts?q=rust'

# Sort by one or more fields
curl 'http://localhost:3000/posts?_sort=views&_order=desc'
curl 'http://localhost:3000/posts?_sort=author,views&_order=asc,desc'

# Offset pagination (returns an array)
curl 'http://localhost:3000/posts?_start=0&_limit=10'

# Page pagination (returns a pagination envelope)
curl 'http://localhost:3000/posts?_page=1&_per_page=10'
```

Page responses include `first`, `prev`, `next`, `last`, `pages`, `items`, and
`data`. Collection responses also include a filtered `X-Total-Count` header.

## Behavior notes

- IDs may be numbers or strings. New numeric IDs use the highest existing ID + 1;
  when string IDs are present, new IDs are UUIDs.
- `POST` with an existing `id` returns `409 Conflict`.
- `PUT` replaces the whole object but keeps the original `id`.
- `PATCH` is a shallow merge and ignores `id` changes.
- Unknown resources and missing items return `404`.
- Using collection-style writes on a singular resource (or vice versa) returns `405`.
- Invalid JSON or a non-object root fails startup or returns `400`.
- CORS is permissive, which is convenient for local development but should be
  tightened before exposing the server publicly.
- The file is re-read only at startup; edits made directly to the file while the
  server runs are not picked up.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- db.json
```

Set `RUST_LOG` to change log verbosity:

```bash
RUST_LOG=rustjs_server=debug,tower_http=debug cargo run -- db.json
```
