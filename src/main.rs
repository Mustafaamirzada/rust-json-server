mod db;
mod error;
mod handlers;
mod query;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{Router, routing::get};
use clap::Parser;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::Database;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<RwLock<Database>>,
}

#[derive(Debug, Parser)]
#[command(
    name = "rustjs-server",
    version,
    about = "Serve a JSON file as a REST API"
)]
struct Cli {
    /// Path to the JSON database file
    #[arg(default_value = "db.json")]
    file: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// Address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustjs_server=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    let database = Database::load(cli.file.clone()).await?;
    let state = AppState {
        db: Arc::new(RwLock::new(database)),
    };

    let app = Router::new()
        .route("/", get(handlers::info))
        .route("/db", get(handlers::full_db))
        .route("/health", get(handlers::health))
        .route(
            "/{resource}",
            get(handlers::list)
                .post(handlers::create)
                .put(handlers::put_singular)
                .patch(handlers::patch_singular),
        )
        .route(
            "/{resource}/{id}",
            get(handlers::get_one)
                .put(handlers::update)
                .patch(handlers::patch)
                .delete(handlers::delete_one),
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let address: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;

    tracing::info!("serving '{}' at http://{address}", cli.file.display());

    axum::serve(listener, app).await?;

    Ok(())
}
